<#
.SYNOPSIS
Builds an unsigned per-user installer from the current local SafeBrowse source.
.DESCRIPTION
Requires Inno Setup 7.1 or newer. Validates the compiler and Microsoft bootstrapper
signatures, rebuilds both application binaries, checks PE metadata, and records hashes.
Does not install SafeBrowse, alter browsing data, create a GitHub repository, or publish.
This local builder accepts an uncommitted checkout; provenance says so explicitly.
#>
[CmdletBinding()]
param(
    [string] $CompilerPath,
    [string] $RuntimeBootstrapperPath,
    [string] $RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [switch] $RequireClean
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$minimumCompilerVersion = [version]'7.1.0'
$bootstrapperUrl = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703'
$runtimeRequiredExitCode = 10
$windowsGuiSubsystem = 2

function Assert-PublisherSignature {
    <# .SYNOPSIS Requires a valid Windows trust chain and the expected publisher identity. #>
    param([string] $Path, [string] $SubjectPattern)
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch $SubjectPattern) {
        throw "The file does not have the required trusted publisher signature: $Path ($($signature.Status))."
    }
    return $signature.SignerCertificate.Subject
}

function Get-ExecutableSubsystem {
    <# .SYNOPSIS Reads the PE optional header without executing the program. #>
    param([string] $Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5A4D) {
        throw "Not a Windows executable: $Path"
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 60)
    $subsystemOffset = $peOffset + 24 + 68
    if ($peOffset -lt 64 -or $subsystemOffset + 2 -gt $bytes.Length -or
        [BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x00004550) {
        throw "Invalid Windows executable header: $Path"
    }
    return [BitConverter]::ToUInt16($bytes, $subsystemOffset)
}

function Get-Sha256 {
    <# .SYNOPSIS Returns a canonical digest for the exact file being packaged. #>
    param([string] $Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-PayloadProbe {
    <# .SYNOPSIS Captures a read-only GUI or console probe without allocating a console. #>
    param([string] $Executable, [string[]] $Arguments)
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $startInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($startInfo)
    try {
        $standardOutput = $process.StandardOutput.ReadToEndAsync()
        $standardError = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(10000)) { throw "Read-only build probe did not finish: $Executable" }
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            StdOut = $standardOutput.GetAwaiter().GetResult()
            StdErr = $standardError.GetAwaiter().GetResult()
        }
    } finally {
        $process.Dispose()
    }
}

function Assert-CommittedInstallerSource {
    <# .SYNOPSIS Requires unchanged committed inputs for a public installer build. #>
    param([string] $ExpectedRevision = '')
    $changes = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0 -or $changes.Count -ne 0) {
        throw 'Public installer builds require the complete source to be committed and the working tree to be clean.'
    }
    $ignoredInputs = @(& git ls-files --others --ignored --exclude-standard -- src assets installer scripts tests examples .cargo)
    if ($LASTEXITCODE -ne 0 -or $ignoredInputs.Count -ne 0) {
        throw 'Ignored build inputs must be reviewed before creating a public installer.'
    }
    $revision = & git rev-parse --verify HEAD
    if ($LASTEXITCODE -ne 0 -or ([string]$revision).Trim() -notmatch '^[0-9a-f]{40,64}$') {
        throw 'Cannot identify the committed installer source.'
    }
    if ($ExpectedRevision -and ([string]$revision).Trim() -ne $ExpectedRevision) {
        throw 'The source revision changed during the installer build.'
    }
    return ([string]$revision).Trim()
}

Push-Location -LiteralPath $RepositoryRoot
try {
    $repositoryPath = (Get-Location).Path
    $publicRevision = if ($RequireClean) { Assert-CommittedInstallerSource } else { '' }
    $toolsDirectory = Join-Path $repositoryPath 'target/installer-tools'
    if ([string]::IsNullOrWhiteSpace($CompilerPath)) {
        $compilerCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        $candidates = @(
            (Join-Path $toolsDirectory 'inno-7.1.0/ISCC.exe'),
            (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 7/ISCC.exe'),
            (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 7 (64-bit)/ISCC.exe'),
            'C:/Program Files/Inno Setup 7/ISCC.exe',
            'C:/Program Files/Inno Setup 7 (64-bit)/ISCC.exe'
        )
        if ($null -ne $compilerCommand) { $candidates = @($compilerCommand.Source) + $candidates }
        $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
    if ([string]::IsNullOrWhiteSpace($CompilerPath)) {
        throw 'Install Inno Setup 7.1 or newer from https://jrsoftware.org/isdl.php, then rerun with -CompilerPath pointing to ISCC.exe. No SafeBrowse installer has been produced.'
    }
    $CompilerPath = (Resolve-Path -LiteralPath $CompilerPath).Path
    $compilerPublisher = Assert-PublisherSignature $CompilerPath '(?:^|, )CN=Pyrsys B\.V\.(?:,|$)'
    # Inno's command-line executable uses zeroed PE versions; query the verified engine.
    $compilerVersionOutput = & $CompilerPath --version
    if ($LASTEXITCODE -ne 0) { throw 'Cannot query the Inno Setup compiler engine version.' }
    $compilerVersion = [version](([string]($compilerVersionOutput -join '')).Trim())
    if ($compilerVersion -lt $minimumCompilerVersion) { throw 'Inno Setup 7.1 or newer is required.' }

    if ([string]::IsNullOrWhiteSpace($RuntimeBootstrapperPath)) {
        [IO.Directory]::CreateDirectory($toolsDirectory) | Out-Null
        $RuntimeBootstrapperPath = Join-Path $toolsDirectory 'MicrosoftEdgeWebview2Setup.exe'
        if (!(Test-Path -LiteralPath $RuntimeBootstrapperPath -PathType Leaf)) {
            Invoke-WebRequest -Uri $bootstrapperUrl -OutFile $RuntimeBootstrapperPath
        }
    }
    $RuntimeBootstrapperPath = (Resolve-Path -LiteralPath $RuntimeBootstrapperPath).Path
    $runtimePublisher = Assert-PublisherSignature $RuntimeBootstrapperPath '(?:^|, )O=Microsoft Corporation(?:,|$)'

    $metadataText = & cargo metadata --format-version 1 --no-deps --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataText | ConvertFrom-Json
    $package = $metadata.packages | Where-Object name -EQ 'safebrowse'
    $version = [string]$package.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Installer versions must have three numeric components.' }
    $runtimeSource = Get-Content -LiteralPath src/browser/runtime.rs -Raw
    $runtimeMatch = [regex]::Match($runtimeSource, 'pub const MINIMUM_SUPPORTED_RUNTIME: &str = "(?<version>\d+\.\d+\.\d+\.\d+)";')
    if (!$runtimeMatch.Success) { throw 'Cannot determine the application runtime support floor.' }
    $minimumRuntime = $runtimeMatch.Groups['version'].Value
    $buildDirectory = Join-Path $repositoryPath 'target/installer-app'
    & cargo build --release --locked --offline --bins --target-dir $buildDirectory
    if ($LASTEXITCODE -ne 0) { throw 'SafeBrowse release build failed.' }
    $binaryDirectory = Join-Path $buildDirectory 'release'
    $application = Join-Path $binaryDirectory 'safebrowse.exe'
    $maintenance = Join-Path $binaryDirectory 'safebrowse-maintenance.exe'
    if ((Get-ExecutableSubsystem $application) -ne $windowsGuiSubsystem) { throw 'Production SafeBrowse must use the Windows GUI subsystem.' }
    if ((Get-Item -LiteralPath $application).VersionInfo.ProductVersion -ne $version) {
        throw 'Executable and installer product versions differ.'
    }
    $helpProbe = Invoke-PayloadProbe $application @('--help')
    if ($helpProbe.ExitCode -ne 0 -or $helpProbe.StdOut -notmatch '(?m)^USAGE:\r?$') { throw 'Application help smoke check failed.' }
    $runtimeProbe = Invoke-PayloadProbe $maintenance @('check-runtime')
    if ($runtimeProbe.ExitCode -ne 0 -and $runtimeProbe.ExitCode -ne $runtimeRequiredExitCode) {
        throw "The build host runtime preflight encountered a blocked configuration: $($runtimeProbe.StdErr)"
    }

    $buildId = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $outputDirectory = Join-Path $repositoryPath "target/installer/$buildId"
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
    $compilerArguments = @(
        '--no-ide-signtools', '--no-signing',
        "--define=RootDirectory=$repositoryPath", "--define=AppBinaryDirectory=$binaryDirectory",
        "--define=RuntimeBootstrapperPath=$RuntimeBootstrapperPath", "--define=OutputDirectory=$outputDirectory",
        "--define=AppVersion=$version", "--define=MinimumRuntimeVersion=$minimumRuntime",
        (Join-Path $repositoryPath 'installer/SafeBrowse.iss')
    )
    & $CompilerPath @compilerArguments
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
    $installerPath = Join-Path $outputDirectory "SafeBrowse-Setup-$version-x64.exe"
    $installerSignature = Get-AuthenticodeSignature -LiteralPath $installerPath
    if ($installerSignature.Status -ne 'NotSigned') { throw 'This build was requested as an unsigned installer.' }
    $revision = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record source revision.' }
    $workingTree = @(& git status --porcelain=v1 --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record working-tree status.' }
    $sourceFiles = @(& rg --files --hidden src assets installer scripts .github tests examples docs -g '!*.sha256') +
        @('Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'README.md', 'CONTRIBUTING.md', 'SECURITY.md', '.gitignore', '.gitattributes')
    $sourceHashes = foreach ($sourceFile in $sourceFiles | Sort-Object -Unique) {
        [ordered]@{ path = $sourceFile.Replace('\', '/'); sha256 = Get-Sha256 $sourceFile }
    }
    $provenance = [ordered]@{
        artifactKind = if ($RequireClean) { 'unsigned-committed-source-installer' } else { 'unsigned-local-working-tree-installer' }
        builtAtUtc = [DateTime]::UtcNow.ToString('o')
        version = $version; sourceRevision = ([string]$revision).Trim(); workingTreeDirty = $workingTree.Count -ne 0
        installer = [ordered]@{ file = [IO.Path]::GetFileName($installerPath); sha256 = Get-Sha256 $installerPath }
        applicationSha256 = Get-Sha256 $application; maintenanceSha256 = Get-Sha256 $maintenance
        compiler = [ordered]@{ version = $compilerVersion.ToString(); publisher = $compilerPublisher; sha256 = Get-Sha256 $CompilerPath }
        runtimeBootstrapper = [ordered]@{ url = $bootstrapperUrl; publisher = $runtimePublisher; sha256 = Get-Sha256 $RuntimeBootstrapperPath }
        sourceFiles = @($sourceHashes)
    }
    if ($RequireClean) { Assert-CommittedInstallerSource $publicRevision | Out-Null }
    [IO.File]::WriteAllText((Join-Path $outputDirectory 'provenance.json'), ($provenance | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $outputDirectory 'SHA256SUMS.txt'), ((Get-Sha256 $installerPath) + '  ' + [IO.Path]::GetFileName($installerPath) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
    Write-Host "Unsigned installer: $installerPath"
    Write-Host 'No installation or publication was performed. Verify installation and removal before distribution.'
} finally {
    Pop-Location
}
