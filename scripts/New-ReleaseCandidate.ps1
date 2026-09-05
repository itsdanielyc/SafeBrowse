<#
.SYNOPSIS
Builds an unsigned candidate from an isolated export of a clean committed revision.
.DESCRIPTION
Never commits, publishes, overwrites candidates, or supplies a signing identity.
ValidateOnly checks source readiness without building or writing candidate files.
Run the checks documented in docs/RELEASING.md before accepting a candidate.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [switch] $ValidateOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$targetTriple = 'x86_64-pc-windows-msvc'
$toolchainVersion = '1.97.0'
$requiredFiles = @(
    'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml',
    'src/main.rs', 'src/lib.rs', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'README.md', 'SECURITY.md',
    'docs/RELEASING.md', 'scripts/New-ReleaseCandidate.ps1'
)

function Invoke-GitRead {
    <# .SYNOPSIS Runs a read-only Git query and fails on an incomplete repository. #>
    param([Parameter(Mandatory)] [string[]] $Arguments)
    $result = & git @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Git query failed: git $($Arguments -join ' ')" }
    return $result
}

function Assert-CleanRevision {
    <# .SYNOPSIS Rejects changes, incomplete source tracking, and ignored build inputs. #>
    $changes = @(Invoke-GitRead -Arguments @('status', '--porcelain=v1', '--untracked-files=all'))
    if ($changes.Count -gt 0) {
        throw 'The complete source must be committed before building a candidate; tracked or untracked changes remain.'
    }
    foreach ($requiredFile in $requiredFiles) {
        $tracked = Invoke-GitRead -Arguments @('ls-files', '--error-unmatch', '--', $requiredFile)
        if ([string]::IsNullOrWhiteSpace(($tracked -join '')) -or !(Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            throw "Required release input is not a tracked file: $requiredFile"
        }
    }
    $ignoredInputs = @(Invoke-GitRead -Arguments @(
        'ls-files', '--others', '--ignored', '--exclude-standard', '--',
        'src', 'assets', 'tests', 'examples', '.cargo', 'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml'
    ))
    if ($ignoredInputs.Count -gt 0) {
        throw 'Ignored files exist inside application input paths; review them before building an identified revision.'
    }
}

Push-Location -LiteralPath $RepositoryRoot
try {
    $repositoryPath = [IO.Path]::GetFullPath((Invoke-GitRead -Arguments @('rev-parse', '--show-toplevel')))
    if (![StringComparer]::OrdinalIgnoreCase.Equals($repositoryPath.TrimEnd('\', '/'), (Get-Location).Path.TrimEnd('\', '/'))) {
        throw 'RepositoryRoot must identify the repository root, not a nested directory.'
    }
    Assert-CleanRevision
    $revision = (Invoke-GitRead -Arguments @('rev-parse', '--verify', 'HEAD')).Trim()
    if ($revision -notmatch '^[a-f0-9]{40,64}$') { throw 'Git did not return a complete source revision.' }
    foreach ($override in @('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
        'CARGO_BUILD_RUSTFLAGS', 'CARGO_BUILD_RUSTC', 'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER')) {
        if (![string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($override))) {
            throw "Remove the unrecorded build override $override before building a release candidate."
        }
    }
    $candidateRoot = Join-Path $repositoryPath 'target/release-candidates'
    foreach ($directory in @((Join-Path $repositoryPath 'target'), $candidateRoot)) {
        if (Test-Path -LiteralPath $directory) {
            $item = Get-Item -LiteralPath $directory -Force
            if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "Candidate output must be an ordinary directory inside the repository: $directory"
            }
        }
    }
    if ($ValidateOnly) {
        Write-Host "Release source validation passed: $revision"
        return
    }
    if (!$IsWindows) { throw 'Release candidates require Windows and the MSVC/Windows SDK build tools.' }

    $candidateName = "safebrowse-$($revision.Substring(0, 12))-$([Guid]::NewGuid().ToString('N'))"
    $candidateDirectory = Join-Path $candidateRoot $candidateName
    [void] [IO.Directory]::CreateDirectory($candidateDirectory)
    $sourceArchive = Join-Path $candidateDirectory 'source.zip'
    & git archive --format=zip --output=$sourceArchive $revision
    if ($LASTEXITCODE -ne 0) { throw 'Could not export the identified Git revision.' }
    $sourceDirectory = Join-Path $candidateDirectory 'source'
    Expand-Archive -LiteralPath $sourceArchive -DestinationPath $sourceDirectory
    $buildDirectory = Join-Path $candidateDirectory 'build'
    $packageDirectory = Join-Path $candidateDirectory 'package'
    [void] [IO.Directory]::CreateDirectory($packageDirectory)

    Push-Location -LiteralPath $sourceDirectory
    try {
        & cargo "+$toolchainVersion" build --release --locked --target $targetTriple --target-dir $buildDirectory
        if ($LASTEXITCODE -ne 0) { throw 'The isolated release build failed.' }
        $compilerVersion = & rustc "+$toolchainVersion" --version --verbose
        if ($LASTEXITCODE -ne 0) { throw 'Could not record the compiler identity.' }
        $cargoVersion = & cargo "+$toolchainVersion" --version
        if ($LASTEXITCODE -ne 0) { throw 'Could not record the Cargo identity.' }
        $metadataJson = & cargo "+$toolchainVersion" metadata --locked --offline --format-version 1 --no-deps
        if ($LASTEXITCODE -ne 0) { throw 'Could not record the package identity.' }
        $packageMetadata = ($metadataJson | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'safebrowse' }
    } finally {
        Pop-Location
    }
    Assert-CleanRevision
    if ((Invoke-GitRead -Arguments @('rev-parse', 'HEAD')).Trim() -ne $revision) {
        throw 'The source revision changed while building; discard this candidate.'
    }

    $executablePath = Join-Path $buildDirectory "$targetTriple/release/safebrowse.exe"
    Copy-Item -LiteralPath $executablePath -Destination (Join-Path $packageDirectory 'safebrowse.exe')
    foreach ($document in @('LICENSE', 'THIRD_PARTY_NOTICES.txt', 'README.md', 'SECURITY.md')) {
        Copy-Item -LiteralPath (Join-Path $sourceDirectory $document) -Destination $packageDirectory
    }
    $packageDocs = Join-Path $packageDirectory 'docs'
    [IO.Directory]::CreateDirectory($packageDocs) | Out-Null
    # Promotional videos remain in the repository without inflating portable packages.
    Get-ChildItem -LiteralPath (Join-Path $sourceDirectory 'docs') | Where-Object Name -NE 'media' |
        Copy-Item -Destination $packageDocs -Recurse
    $signature = Get-AuthenticodeSignature -LiteralPath $executablePath
    $provenance = [ordered]@{
        schema_version = 1
        product = 'SafeBrowse'
        version = $packageMetadata.version
        source_revision = $revision
        source_export = 'git archive of the clean committed revision'
        source_archive_sha256 = (Get-FileHash -LiteralPath $sourceArchive -Algorithm SHA256).Hash.ToLowerInvariant()
        cargo_lock_sha256 = (Get-FileHash -LiteralPath (Join-Path $sourceDirectory 'Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
        target = $targetTriple
        rustc = @($compilerVersion)
        cargo = ($cargoVersion -join ' ')
        windows_version = [Environment]::OSVersion.VersionString
        executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
        authenticode_status = $signature.Status.ToString()
        built_at_utc = [DateTime]::UtcNow.ToString('o')
        webview2 = 'Installed separately; runtime version and updates are not covered by this source manifest.'
        validation = 'Build success only; consult the matching CI run and manual release record.'
        github_attestation = 'Available only if a matching GitHub candidate workflow completes its attestation job.'
    }
    $provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $packageDirectory 'provenance.json') -Encoding utf8NoBOM
    $distributionDirectory = Join-Path $candidateDirectory 'distribution'
    [void] [IO.Directory]::CreateDirectory($distributionDirectory)
    $zipPath = Join-Path $distributionDirectory "safebrowse-$($packageMetadata.version)-windows-x64-$($revision.Substring(0, 12)).zip"
    Compress-Archive -Path (Join-Path $packageDirectory '*') -DestinationPath $zipPath
    $zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$zipHash  $([IO.Path]::GetFileName($zipPath))" | Set-Content -LiteralPath (Join-Path $distributionDirectory 'SHA256SUMS.txt') -Encoding ascii
    Copy-Item -LiteralPath (Join-Path $packageDirectory 'provenance.json') -Destination $distributionDirectory
    if (![string]::IsNullOrEmpty($env:GITHUB_OUTPUT)) {
        "distribution-directory=$distributionDirectory" | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8NoBOM
    }
    Write-Host "Candidate: $zipPath"
    Write-Host "Authenticode status: $($signature.Status). No publication was performed."
} finally {
    Pop-Location
}
