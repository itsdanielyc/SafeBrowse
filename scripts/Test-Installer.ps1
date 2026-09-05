<#
.SYNOPSIS
Compiles and exercises the installer with isolated, offline native fixtures.
.DESCRIPTION
Copies the production Inno script with guarded substitutions for a unique AppId,
destination, shortcut and mutex names. Every packaged executable is a compiled
mock fixed to the disposable root; no real app, cleanup or Microsoft installer runs.
Leaves fixture sources, files and process logs under target/installer-tests.
Requires a non-elevated Windows session and Inno Setup 7.1 or newer.
#>
[CmdletBinding()]
param(
    [string] $CompilerPath = (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 7/ISCC.exe'),
    [int] $ProcessTimeoutSeconds = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryPath = Split-Path -Parent $PSScriptRoot
$fixtureId = [Guid]::NewGuid().ToString('N')
$fixtureRoot = Join-Path $repositoryPath "target/installer-tests/$fixtureId"
$installDirectory = Join-Path $fixtureRoot 'installed'
$binaryDirectory = Join-Path $fixtureRoot 'binaries'
$fixtureName = "SafeBrowse Installer Test $fixtureId"
$fixtureAppId = [Guid]::NewGuid().ToString()
$registrySubkey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\{$fixtureAppId}_is1"
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$fixtureName.lnk"
$results = [Collections.Generic.List[object]]::new()
$utf8 = [Text.UTF8Encoding]::new($false)

function Assert-Condition {
    <# .SYNOPSIS Stops the suite before another mutation if an invariant fails. #>
    param([bool] $Condition, [string] $Message)
    if (!$Condition) { throw $Message }
}

function Replace-Expected {
    <# .SYNOPSIS Requires exact source matches so production changes fail visibly. #>
    param([string] $Source, [string] $Old, [string] $New, [int] $Count = 1)
    $actualCount = [regex]::Matches($Source, [regex]::Escape($Old)).Count
    if ($actualCount -ne $Count) { throw "Expected $Count matches, found $actualCount for fixture replacement: $Old" }
    return $Source.Replace($Old, $New)
}

function Set-FixtureResponse {
    <# .SYNOPSIS Configures a mock exit code without modifying environment or runtime. #>
    param([string] $Name, [int] $ExitCode)
    [IO.File]::WriteAllText((Join-Path $fixtureRoot "$Name-exit.txt"), [string]$ExitCode, $utf8)
}

function Get-FixtureCalls {
    <# .SYNOPSIS Reads the mocked native call history, including failed prerequisites. #>
    $log = Join-Path $fixtureRoot 'calls.log'
    if (Test-Path -LiteralPath $log) { return @(Get-Content -LiteralPath $log) }
    return @()
}

function Invoke-FixtureProcess {
    <# .SYNOPSIS Runs a hidden fixture process with an explicit bound and retained log. #>
    param([string] $Path, [string] $Case, [string[]] $ExtraArguments = @())
    $resolvedPath = (Resolve-Path -LiteralPath $Path).Path
    Assert-Condition ($resolvedPath.StartsWith($fixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) 'Refusing to execute a non-fixture installer.'
    $logPath = Join-Path $fixtureRoot "$Case.log"
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', ('/LOG="' + $logPath + '"')) + $ExtraArguments
    $process = Start-Process -FilePath $resolvedPath -ArgumentList $arguments -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit($ProcessTimeoutSeconds * 1000)) {
        $process.Kill($true)
        throw "Fixture process exceeded $ProcessTimeoutSeconds seconds: $Case. Only this process tree was stopped."
    }
    $process.Refresh()
    return $process.ExitCode
}

function Assert-Installed {
    <# .SYNOPSIS Confirms fixture files, shortcut and uninstall registration exist. #>
    foreach ($name in @('safebrowse.exe', 'safebrowse-maintenance.exe', 'unins000.exe')) {
        Assert-Condition (Test-Path -LiteralPath (Join-Path $installDirectory $name) -PathType Leaf) "Missing installed fixture file: $name"
    }
    Assert-Condition (Test-Path -LiteralPath $shortcutPath -PathType Leaf) 'Fixture Start menu shortcut is missing.'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubkey)
    Assert-Condition ($null -ne $key) 'Fixture uninstall registration is missing.'
    $key.Dispose()
}

function Assert-Removed {
    <# .SYNOPSIS Confirms successful removal did not strand app registration or files. #>
    # Inno's temporary second phase finishes self-deletion after the original exits.
    $uninstallerPath = Join-Path $installDirectory 'unins000.exe'
    $selfRemovalDeadline = [DateTime]::UtcNow.AddSeconds(10)
    while ((Test-Path -LiteralPath $uninstallerPath) -and [DateTime]::UtcNow -lt $selfRemovalDeadline) {
        Start-Sleep -Milliseconds 100
    }
    foreach ($name in @('safebrowse.exe', 'safebrowse-maintenance.exe', 'unins000.exe')) {
        Assert-Condition (!(Test-Path -LiteralPath (Join-Path $installDirectory $name))) "Fixture removal left $name"
    }
    Assert-Condition (!(Test-Path -LiteralPath $shortcutPath)) 'Fixture removal left its Start menu shortcut.'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubkey)
    if ($null -ne $key) { $key.Dispose(); throw 'Fixture removal left its uninstall registration.' }
}

function Record-Pass {
    <# .SYNOPSIS Records each completed scenario for reproducible validation reports. #>
    param([string] $Case, [int] $ExitCode)
    $results.Add([ordered]@{ case = $Case; exitCode = $ExitCode; passed = $true })
    Write-Host "Passed $Case (exit $ExitCode)."
}

function Build-FixtureInstaller {
    <# .SYNOPSIS Compiles a production-derived script with only isolated mock inputs. #>
    param([string] $Variant, [string] $Source)
    $outputDirectory = Join-Path $fixtureRoot $Variant
    [void][IO.Directory]::CreateDirectory($outputDirectory)
    $sourcePath = Join-Path $outputDirectory 'SafeBrowse.iss'
    [IO.File]::WriteAllText($sourcePath, $Source, $utf8)
    $arguments = @(
        '--no-ide-signtools', '--no-signing',
        "--define=RootDirectory=$repositoryPath", "--define=AppBinaryDirectory=$binaryDirectory",
        "--define=RuntimeBootstrapperPath=$(Join-Path $binaryDirectory 'MicrosoftEdgeWebview2Setup.exe')",
        "--define=OutputDirectory=$outputDirectory", '--define=AppVersion=1.0.0', '--define=MinimumRuntimeVersion=151.0.4129.107', $sourcePath
    )
    & $CompilerPath @arguments *> (Join-Path $outputDirectory 'compile.log')
    if ($LASTEXITCODE -ne 0) { throw "Fixture compiler failed; see $outputDirectory/compile.log" }
    return Join-Path $outputDirectory 'SafeBrowse-Setup-1.0.0-x64.exe'
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
Assert-Condition (!([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Run installer tests without administrator privileges.'
Assert-Condition ($ProcessTimeoutSeconds -gt 0 -and $ProcessTimeoutSeconds -le 60) 'Fixture process timeout must be 1–60 seconds.'
$CompilerPath = (Resolve-Path -LiteralPath $CompilerPath).Path
$signature = Get-AuthenticodeSignature -LiteralPath $CompilerPath
Assert-Condition ($signature.Status -eq 'Valid' -and $signature.SignerCertificate.Subject -match '(?:^|, )CN=Pyrsys B\.V\.(?:,|$)') 'The compiler must have a valid Pyrsys publisher signature.'
[void][IO.Directory]::CreateDirectory($binaryDirectory)
$fixtureRoot = (Resolve-Path -LiteralPath $fixtureRoot).Path
Assert-Condition ($fixtureRoot.StartsWith((Join-Path $repositoryPath 'target/installer-tests') + '\', [StringComparison]::OrdinalIgnoreCase)) 'Unexpected fixture directory.'
[IO.File]::WriteAllText((Join-Path $fixtureRoot 'fixture-id.txt'), $fixtureId, $utf8)
[IO.File]::WriteAllText((Join-Path $fixtureRoot 'user-data.txt'), 'Disposable mock user data', $utf8)
Set-FixtureResponse 'runtime' 0
Set-FixtureResponse 'cleanup' 0
Set-FixtureResponse 'bootstrap' 7

$previousFixtureRoot = $env:SAFEBROWSE_INSTALLER_FIXTURE_ROOT
$previousFixtureId = $env:SAFEBROWSE_INSTALLER_FIXTURE_ID
try {
    $env:SAFEBROWSE_INSTALLER_FIXTURE_ROOT = $fixtureRoot
    $env:SAFEBROWSE_INSTALLER_FIXTURE_ID = $fixtureId
    & rustc --edition=2021 (Join-Path $repositoryPath 'tests/fixtures/installer_maintenance.rs') -o (Join-Path $binaryDirectory 'safebrowse-maintenance.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Rust fixture compilation failed.' }
} finally {
    $env:SAFEBROWSE_INSTALLER_FIXTURE_ROOT = $previousFixtureRoot
    $env:SAFEBROWSE_INSTALLER_FIXTURE_ID = $previousFixtureId
}
foreach ($name in @('safebrowse.exe', 'MicrosoftEdgeWebview2Setup.exe')) {
    Copy-Item -LiteralPath (Join-Path $binaryDirectory 'safebrowse-maintenance.exe') -Destination (Join-Path $binaryDirectory $name)
}

$productionSourcePath = Join-Path $repositoryPath 'installer/SafeBrowse.iss'
$source = Get-Content -LiteralPath $productionSourcePath -Raw
$sourceDigest = (Get-FileHash -LiteralPath $productionSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$source = Replace-Expected $source 'fd54b727-cea2-44b9-8c39-25c704348aea' $fixtureAppId
$source = Replace-Expected $source 'AppName=SafeBrowse' "AppName=$fixtureName"
$source = Replace-Expected $source 'DefaultGroupName=SafeBrowse' "DefaultGroupName=$fixtureName"
$source = Replace-Expected $source 'UninstallDisplayName=SafeBrowse' "UninstallDisplayName=$fixtureName"
$source = Replace-Expected $source '{localappdata}\Programs\SafeBrowse' $installDirectory 2
$source = Replace-Expected $source '{userprograms}\SafeBrowse' "{userprograms}\$fixtureName"
$source = Replace-Expected $source '{userdesktop}\SafeBrowse' "{userdesktop}\$fixtureName"
$source = Replace-Expected $source 'Local\SafeBrowse_Session_Mutex' "Local\SafeBrowse_Test_${fixtureId}_Session" 2
$source = Replace-Expected $source 'Local\SafeBrowse_Setup_Mutex' "Local\SafeBrowse_Test_${fixtureId}_Setup"
$source = Replace-Expected $source 'Local\SafeBrowse_Maintenance_Mutex' "Local\SafeBrowse_Test_${fixtureId}_Maintenance"
$normalInstaller = Build-FixtureInstaller 'normal' $source

try {
    Set-FixtureResponse 'runtime' 1
    $code = Invoke-FixtureProcess $normalInstaller 'blocked-runtime'
    Assert-Condition ($code -ne 0) 'A blocked runtime configuration must stop setup.'
    Assert-Condition (@(Get-FixtureCalls) -contains 'safebrowse-maintenance.exe check-runtime') 'Runtime preflight did not execute.'
    Assert-Condition (@(Get-FixtureCalls | Where-Object { $_ -like 'MicrosoftEdgeWebview2Setup.exe*' }).Count -eq 0) 'Blocked configuration unexpectedly invoked the bootstrapper.'
    Assert-Removed
    Record-Pass 'blocked-runtime' $code

    Set-FixtureResponse 'runtime' 10
    $code = Invoke-FixtureProcess $normalInstaller 'missing-runtime-bootstrap-failure'
    Assert-Condition ($code -ne 0) 'A failed bootstrapper must stop setup.'
    Assert-Condition (@(Get-FixtureCalls | Where-Object { $_ -eq 'MicrosoftEdgeWebview2Setup.exe /silent /install' }).Count -eq 1) 'The runtime-required result did not invoke the mock bootstrapper exactly once.'
    Assert-Condition ((Get-Content -LiteralPath (Join-Path $fixtureRoot 'missing-runtime-bootstrap-failure.log') -Raw).Contains('exit code 7')) 'Bootstrap failure diagnostic lost its exit code.'
    Assert-Removed
    Record-Pass 'missing-runtime-bootstrap-failure' $code

    Set-FixtureResponse 'runtime' 0
    $code = Invoke-FixtureProcess $normalInstaller 'initial-install'
    Assert-Condition ($code -eq 0) 'Fixture initial installation failed.'
    Assert-Installed
    Record-Pass 'initial-install' $code

    $code = Invoke-FixtureProcess $normalInstaller 'same-version-reinstall'
    Assert-Condition ($code -eq 0) 'Same-version reinstall failed.'
    Assert-Installed
    Record-Pass 'same-version-reinstall' $code

    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubkey, $true)
    try { $key.SetValue('DisplayVersion', '2.0.0') } finally { $key.Dispose() }
    try {
        $callsBefore = @(Get-FixtureCalls).Count
        $code = Invoke-FixtureProcess $normalInstaller 'downgrade-rejected'
        Assert-Condition ($code -ne 0) 'An installer downgrade was not rejected.'
        Assert-Condition (@(Get-FixtureCalls).Count -eq $callsBefore) 'Downgrade rejection occurred after executing maintenance.'
        Assert-Installed
        Record-Pass 'downgrade-rejected' $code
    } finally {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registrySubkey, $true)
        try { $key.SetValue('DisplayVersion', '1.0.0') } finally { $key.Dispose() }
    }

    $code = Invoke-FixtureProcess $normalInstaller 'custom-destination-rejected' @('/DIR="' + (Join-Path $fixtureRoot 'other-destination') + '"')
    Assert-Condition ($code -ne 0) 'An unexpected installation destination was not rejected.'
    Assert-Condition (!(Test-Path -LiteralPath (Join-Path $fixtureRoot 'other-destination'))) 'Setup wrote into the rejected destination.'
    Assert-Installed
    Record-Pass 'custom-destination-rejected' $code

    Set-FixtureResponse 'cleanup' 1
    $code = Invoke-FixtureProcess (Join-Path $installDirectory 'unins000.exe') 'cleanup-failure-keeps-application'
    Assert-Condition ($code -ne 0) 'Cleanup failure did not fail removal.'
    Assert-Installed
    Assert-Condition (Test-Path -LiteralPath (Join-Path $fixtureRoot 'user-data.txt')) 'Failed cleanup removed fixture data.'
    Record-Pass 'cleanup-failure-keeps-application' $code

    Set-FixtureResponse 'cleanup' 0
    $code = Invoke-FixtureProcess (Join-Path $installDirectory 'unins000.exe') 'silent-removal-preserves-data'
    Assert-Condition ($code -eq 0) 'Silent fixture removal failed.'
    Assert-Removed
    Assert-Condition (Test-Path -LiteralPath (Join-Path $fixtureRoot 'user-data.txt')) 'Silent removal unexpectedly opted into deleting user data.'
    Assert-Condition (@(Get-FixtureCalls)[-1] -eq 'safebrowse-maintenance.exe cleanup') 'Silent removal passed unexpected cleanup arguments.'
    Record-Pass 'silent-removal-preserves-data' $code

    # Reach the production prompt under silent execution to verify its IDNO default.
    $promptSource = Replace-Expected $source 'if not UninstallSilent then begin' 'if True then begin'
    $noInstaller = Build-FixtureInstaller 'prompt-no-default' $promptSource
    $code = Invoke-FixtureProcess $noInstaller 'install-for-prompt-default'
    Assert-Condition ($code -eq 0) 'Prompt-default fixture install failed.'
    $code = Invoke-FixtureProcess (Join-Path $installDirectory 'unins000.exe') 'prompt-no-default-preserves-data'
    Assert-Condition ($code -eq 0) 'Default-No removal failed.'
    Assert-Removed
    Assert-Condition (Test-Path -LiteralPath (Join-Path $fixtureRoot 'user-data.txt')) 'The prompt default deleted user data.'
    Assert-Condition (@(Get-FixtureCalls)[-1] -eq 'safebrowse-maintenance.exe cleanup') 'Default-No removal passed unexpected cleanup arguments.'
    Record-Pass 'prompt-no-default-preserves-data' $code

    $choicePattern = '(?m)^    Choice := SuppressibleMsgBox\([^\r\n]+\);$'
    $choiceMatches = [regex]::Matches($promptSource, $choicePattern)
    Assert-Condition ($choiceMatches.Count -eq 1) 'Cannot isolate the production removal-data prompt.'
    $cancelSource = Replace-Expected $promptSource $choiceMatches[0].Value '    Choice := IDCANCEL;'
    $cancelInstaller = Build-FixtureInstaller 'prompt-cancel' $cancelSource
    $code = Invoke-FixtureProcess $cancelInstaller 'install-for-prompt-cancel'
    Assert-Condition ($code -eq 0) 'Cancel fixture install failed.'
    $callsBefore = @(Get-FixtureCalls).Count
    $code = Invoke-FixtureProcess (Join-Path $installDirectory 'unins000.exe') 'prompt-cancel-keeps-application'
    Assert-Condition ($code -ne 0) 'Cancel did not abort removal.'
    Assert-Installed
    Assert-Condition (@(Get-FixtureCalls).Count -eq $callsBefore) 'Cancelled removal executed cleanup.'
    Record-Pass 'prompt-cancel-keeps-application' $code

    $yesSource = Replace-Expected $promptSource $choiceMatches[0].Value '    Choice := IDYES;'
    $yesInstaller = Build-FixtureInstaller 'prompt-yes' $yesSource
    $code = Invoke-FixtureProcess $yesInstaller 'install-for-prompt-yes'
    Assert-Condition ($code -eq 0) 'Yes fixture install failed.'
    $code = Invoke-FixtureProcess (Join-Path $installDirectory 'unins000.exe') 'explicit-data-removal'
    Assert-Condition ($code -eq 0) 'Explicit data removal failed.'
    Assert-Removed
    Assert-Condition (!(Test-Path -LiteralPath (Join-Path $fixtureRoot 'user-data.txt'))) 'Explicit removal kept the mock user data.'
    Assert-Condition (@(Get-FixtureCalls)[-1] -eq 'safebrowse-maintenance.exe cleanup --remove-user-data') 'Explicit removal omitted its cleanup opt-in.'
    Record-Pass 'explicit-data-removal' $code
} finally {
    $report = [ordered]@{
        source = 'installer/SafeBrowse.iss'; sourceSha256 = $sourceDigest
        fixtureRoot = $fixtureRoot; fixtureAppId = $fixtureAppId; installDirectory = $installDirectory
        shortcut = $shortcutPath; registrySubkey = $registrySubkey
        mocksOnly = $true; scenarios = @($results.ToArray())
        limitations = @('Real WebView2 installation and application startup are not exercised.', 'Prompt choices use production-derived fixture source; actual visual interaction is not exercised.')
    }
    [IO.File]::WriteAllText((Join-Path $fixtureRoot 'results.json'), ($report | ConvertTo-Json -Depth 5), $utf8)
    Write-Host "Fixture and logs retained at $fixtureRoot"
}
