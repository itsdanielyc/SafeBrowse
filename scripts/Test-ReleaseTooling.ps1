<#
.SYNOPSIS
Exercises release guards, packaging, and audit filtering in a disposable Git fixture.
.DESCRIPTION
Cargo and rustc are mocked within this process: this verifies orchestration, not a
compiled executable. Fixture files stay beneath target/release-tooling-tests.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryPath = Split-Path -Parent $PSScriptRoot
$candidateScript = Join-Path $PSScriptRoot 'New-ReleaseCandidate.ps1'
$auditScript = Join-Path $PSScriptRoot 'Test-WindowsDependencies.ps1'
$fixtureDirectory = Join-Path $repositoryPath "target/release-tooling-tests/$([Guid]::NewGuid().ToString('N'))"
[void] [IO.Directory]::CreateDirectory($fixtureDirectory)
$fixtureFiles = @('Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml', 'src/main.rs', 'src/lib.rs',
    'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'README.md', 'SECURITY.md', 'docs/RELEASING.md', 'scripts/New-ReleaseCandidate.ps1')
foreach ($fixtureFile in $fixtureFiles) {
    $fixturePath = Join-Path $fixtureDirectory $fixtureFile
    [void] [IO.Directory]::CreateDirectory((Split-Path -Parent $fixturePath))
    'release tooling fixture' | Set-Content -LiteralPath $fixturePath -Encoding utf8NoBOM
}
"/target`n/ignored`nsrc/ignored.rs" | Set-Content -LiteralPath (Join-Path $fixtureDirectory '.gitignore') -Encoding utf8NoBOM

function Invoke-FixtureGit {
    <# .SYNOPSIS Runs Git only against the newly created disposable fixture. #>
    param([Parameter(Mandatory)] [string[]] $Arguments)
    & git -C $fixtureDirectory @Arguments | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Fixture Git operation failed.' }
}

function Assert-Rejected {
    <# .SYNOPSIS Requires the given operation to fail with the expected guard reason. #>
    param([Parameter(Mandatory)] [scriptblock] $Operation, [Parameter(Mandatory)] [string] $Reason)
    $rejected = $false
    try { & $Operation } catch {
        if ($_.Exception.Message -notlike "*$Reason*") { throw }
        $rejected = $true
    }
    if (!$rejected) { throw "Expected rejection was missing: $Reason" }
}

Invoke-FixtureGit -Arguments @('init', '--quiet')
Invoke-FixtureGit -Arguments @('add', '.')
Invoke-FixtureGit -Arguments @('-c', 'user.name=Release tooling fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--quiet', '-m', 'Fixture')
& $candidateScript -RepositoryRoot $fixtureDirectory -ValidateOnly
'uncommitted' | Set-Content -LiteralPath (Join-Path $fixtureDirectory 'untracked.rs')
Assert-Rejected -Operation { & $candidateScript -RepositoryRoot $fixtureDirectory -ValidateOnly } -Reason 'complete source must be committed'
Remove-Item -LiteralPath (Join-Path $fixtureDirectory 'untracked.rs')
'ignored source' | Set-Content -LiteralPath (Join-Path $fixtureDirectory 'src/ignored.rs')
Assert-Rejected -Operation { & $candidateScript -RepositoryRoot $fixtureDirectory -ValidateOnly } -Reason 'Ignored files exist'
Remove-Item -LiteralPath (Join-Path $fixtureDirectory 'src/ignored.rs')
'changed' | Add-Content -LiteralPath (Join-Path $fixtureDirectory 'README.md')
Assert-Rejected -Operation { & $candidateScript -RepositoryRoot $fixtureDirectory -ValidateOnly } -Reason 'complete source must be committed'
Invoke-FixtureGit -Arguments @('restore', '--', 'README.md')
Write-Host 'Passed clean, untracked, ignored-source and tracked-change guards.'

$registrySource = 'registry+https://github.com/rust-lang/crates.io-index'
$mockMetadata = @{
    packages = @(
        @{ id = 'safebrowse'; name = 'safebrowse'; version = '0.1.0'; source = $null },
        @{ id = 'windows-package'; name = 'windows-package'; version = '1.0.0'; source = $registrySource },
        @{ id = 'other-platform'; name = 'other-platform'; version = '1.0.0'; source = $registrySource }
    )
    resolve = @{ nodes = @(@{ id = 'safebrowse' }, @{ id = 'windows-package' }) }
} | ConvertTo-Json -Depth 8 -Compress
$mockAuditPackage = 'other-platform'
$mockAuditExitCode = 0

function cargo {
    <# .SYNOPSIS Supplies deterministic command outputs and a clearly fake executable. #>
    param([Parameter(ValueFromRemainingArguments)] [string[]] $Arguments)
    $global:LASTEXITCODE = 0
    if ($Arguments -contains 'build') {
        $targetDirectory = $Arguments[[Array]::IndexOf($Arguments, '--target-dir') + 1]
        $fakeExecutableDirectory = Join-Path $targetDirectory 'x86_64-pc-windows-msvc/release'
        [void] [IO.Directory]::CreateDirectory($fakeExecutableDirectory)
        'NOT AN EXECUTABLE: release orchestration fixture' | Set-Content -LiteralPath (Join-Path $fakeExecutableDirectory 'safebrowse.exe')
        return
    }
    if ($Arguments -contains 'metadata') { return $mockMetadata }
    if ($Arguments -contains 'audit') {
        if ($Arguments -contains '--version') { return 'cargo-audit-audit 0.22.2' }
        $global:LASTEXITCODE = $mockAuditExitCode
        if ($mockAuditExitCode -eq 9) { return '' }
        return (@{
            database = @{ 'last-commit' = 'fixture'; 'last-updated' = '2026-09-04' }
            vulnerabilities = @{ list = @() }
            warnings = @{ unsound = @(@{
                package = @{ name = $mockAuditPackage; version = '1.0.0'; source = $registrySource }
                advisory = @{ id = 'RUSTSEC-FIXTURE'; title = 'Fixture advisory' }
            }) }
        } | ConvertTo-Json -Depth 8 -Compress)
    }
    if ($Arguments -contains '--version') { return 'cargo 1.97.0 (fixture)' }
    throw "Unexpected mocked Cargo invocation: $($Arguments -join ' ')"
}

function rustc {
    <# .SYNOPSIS Returns a fixture identity without invoking the compiler. #>
    param([Parameter(ValueFromRemainingArguments)] [string[]] $Arguments)
    $global:LASTEXITCODE = 0
    return 'rustc 1.97.0 (fixture)'
}

& $auditScript -RepositoryRoot $fixtureDirectory
$auditReport = Get-Content -LiteralPath (Join-Path $fixtureDirectory 'target/windows-dependency-audit.json') -Raw | ConvertFrom-Json
if ($auditReport.applicable_findings.Count -ne 0 -or $auditReport.other_platform_findings.Count -ne 1) {
    throw 'Other-platform advisory was not classified separately.'
}
$mockAuditPackage = 'windows-package'
Assert-Rejected -Operation { & $auditScript -RepositoryRoot $fixtureDirectory } -Reason 'Windows dependency findings'
$mockAuditExitCode = 9
Assert-Rejected -Operation { & $auditScript -RepositoryRoot $fixtureDirectory } -Reason 'usable report'
Write-Host 'Passed Windows advisory filtering and audit-tool failure handling.'

& $candidateScript -RepositoryRoot $fixtureDirectory
$candidateZip = @(Get-ChildItem -LiteralPath (Join-Path $fixtureDirectory 'target/release-candidates') -Filter '*.zip' -File -Recurse |
    Where-Object { $_.Directory.Name -eq 'distribution' })
if ($candidateZip.Count -ne 1) { throw 'Expected exactly one packaged candidate.' }
$manifest = Get-Content -LiteralPath (Join-Path $candidateZip[0].DirectoryName 'provenance.json') -Raw | ConvertFrom-Json
$expectedRevision = (& git -C $fixtureDirectory rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $manifest.source_revision -ne $expectedRevision -or $manifest.cargo -notlike '*fixture*') {
    throw 'Candidate provenance lost the source revision or mocked compiler identity.'
}
$expectedChecksum = (Get-FileHash -LiteralPath $candidateZip[0].FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$checksums = Get-Content -LiteralPath (Join-Path $candidateZip[0].DirectoryName 'SHA256SUMS.txt') -Raw
if (!$checksums.StartsWith($expectedChecksum + '  ')) { throw 'Candidate archive checksum does not match.' }
Write-Host 'Passed isolated export, mocked build, provenance and ZIP checksum checks.'
Write-Host "Disposable fixture retained at $fixtureDirectory"
