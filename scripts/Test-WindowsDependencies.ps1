<#
.SYNOPSIS
Checks RustSec findings against the packages actually resolved for Windows.
.DESCRIPTION
Requires cargo-audit 0.22.2. Fails on vulnerabilities or warnings affecting the
Windows graph, and records other-platform findings separately without hiding them.
The audit covers Rust packages, not the installed WebView2 runtime.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [string] $ReportPath = 'target/windows-dependency-audit.json'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$targetTriple = 'x86_64-pc-windows-msvc'
$requiredAuditVersion = '0.22.2'

Push-Location -LiteralPath $RepositoryRoot
try {
    $auditVersion = & cargo audit --version
    if ($LASTEXITCODE -ne 0 -or $auditVersion -notmatch "\b$([regex]::Escape($requiredAuditVersion))$") {
        throw "Install the reviewed audit tool: cargo install cargo-audit --version $requiredAuditVersion --locked"
    }
    $metadataJson = & cargo metadata --locked --format-version 1 --filter-platform $targetTriple
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the locked Windows dependency graph.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $resolvedIdentifiers = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($node in $metadata.resolve.nodes) { [void] $resolvedIdentifiers.Add($node.id) }
    $windowsPackages = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($package in $metadata.packages) {
        if ($resolvedIdentifiers.Contains($package.id)) {
            [void] $windowsPackages.Add("$($package.name)|$($package.version)|$($package.source)")
        }
    }

    $auditJson = & cargo audit --json --target-os windows --target-arch x86_64
    $auditExitCode = $LASTEXITCODE
    if ($auditExitCode -notin @(0, 1) -or [string]::IsNullOrWhiteSpace(($auditJson -join "`n"))) {
        throw "cargo audit did not return a usable report (exit $auditExitCode)."
    }
    $audit = $auditJson | ConvertFrom-Json
    if ($null -eq $audit.database -or $null -eq $audit.vulnerabilities -or $null -eq $audit.warnings) {
        throw 'cargo audit returned an unexpected report schema.'
    }

    $applicableFindings = [System.Collections.Generic.List[object]]::new()
    $otherPlatformFindings = [System.Collections.Generic.List[object]]::new()
    $findings = [System.Collections.Generic.List[object]]::new()
    foreach ($finding in $audit.vulnerabilities.list) { $findings.Add($finding) }
    foreach ($warningGroup in $audit.warnings.PSObject.Properties) {
        foreach ($finding in $warningGroup.Value) { $findings.Add($finding) }
    }
    # Hash sets keep target filtering linear in the graph plus finding counts.
    foreach ($finding in $findings) {
        $package = $finding.package
        $summary = [ordered]@{
            package = $package.name
            version = $package.version
            advisory = $finding.advisory.id
            title = $finding.advisory.title
        }
        if ($windowsPackages.Contains("$($package.name)|$($package.version)|$($package.source)")) {
            $applicableFindings.Add($summary)
        } else {
            $otherPlatformFindings.Add($summary)
        }
    }

    $report = [ordered]@{
        target = $targetTriple
        audit_tool = ($auditVersion -join ' ')
        database = $audit.database
        resolved_package_count_including_root = $windowsPackages.Count
        applicable_findings = @($applicableFindings.ToArray())
        other_platform_findings = @($otherPlatformFindings.ToArray())
        scope = 'RustSec package advisories only; excludes WebView2 runtime and application-code review.'
    }
    $reportAbsolutePath = [IO.Path]::GetFullPath($ReportPath, (Get-Location).Path)
    [void] [IO.Directory]::CreateDirectory((Split-Path -Parent $reportAbsolutePath))
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportAbsolutePath -Encoding utf8NoBOM
    Write-Host "Windows dependency findings: $($applicableFindings.Count). Other-platform findings: $($otherPlatformFindings.Count)."
    Write-Host "Audit report: $reportAbsolutePath"
    if ($applicableFindings.Count -gt 0) {
        throw 'Review and resolve the Windows dependency findings before accepting this build.'
    }
    if ($auditExitCode -ne 0 -and $findings.Count -eq 0) {
        throw 'cargo audit failed without a classified finding; the audit cannot be accepted.'
    }
} finally {
    Pop-Location
}
