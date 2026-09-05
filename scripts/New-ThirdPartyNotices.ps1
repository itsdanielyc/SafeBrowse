<#
.SYNOPSIS
Collects packaged third-party notices for the locked Windows build.
.DESCRIPTION
Requires PowerShell 7 and an already-fetched Cargo cache. Traverses normal and
build dependencies for x86_64-pc-windows-msvc, excluding development-only edges.
Copies supplied license text without interpreting its terms. Missing texts fail
before replacing the output. Use -Check to verify the checked-in output without
writing it. This is an attribution inventory, not a license audit.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [string] $OutputPath = 'THIRD_PARTY_NOTICES.txt',
    [string] $SupplementManifestPath = 'docs/third-party/supplements.json',
    [switch] $Check
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$targetTriple = 'x86_64-pc-windows-msvc'
$noticeFilenamePattern = '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)(?:$|[._-])'

function Get-ContainedFile {
    <# Rejects paths outside the specified source directory and reparse-point files. #>
    param([string] $Directory, [string] $Path)
    $absolutePath = [IO.Path]::GetFullPath($Path, $Directory)
    $relativePath = [IO.Path]::GetRelativePath($Directory, $absolutePath)
    if ([IO.Path]::IsPathRooted($relativePath) -or $relativePath -match '^\.\.(?:[\\/]|$)') {
        throw "Notice path leaves its source directory: $Path"
    }
    $file = Get-Item -LiteralPath $absolutePath
    if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Notice path is not a regular file: $Path"
    }
    return $file
}

function Get-PackageNoticeFiles {
    <# Returns only supplied notice files inside the package's Cargo directory. #>
    param([Parameter(Mandatory)] [object] $Package)

    $packageDirectory = Split-Path -Parent $Package.manifest_path
    $noticePaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in Get-ChildItem -LiteralPath $packageDirectory -File -Recurse) {
        if ($file.Name -match $noticeFilenamePattern) { [void] $noticePaths.Add($file.FullName) }
    }
    if (-not [string]::IsNullOrWhiteSpace($Package.license_file)) {
        [void] $noticePaths.Add([IO.Path]::GetFullPath($Package.license_file, $packageDirectory))
    }
    foreach ($path in @($noticePaths | Sort-Object -CaseSensitive)) {
        $relativePath = [IO.Path]::GetRelativePath($packageDirectory, $path)
        if ([IO.Path]::IsPathRooted($relativePath) -or $relativePath -match '^\.\.(?:[\\/]|$)') {
            throw "License path leaves package directory: $($Package.name) $($Package.version)"
        }
        $file = Get-ContainedFile -Directory $packageDirectory -Path $path
        [pscustomobject]@{
            Name = $relativePath.Replace('\', '/')
            Text = [IO.File]::ReadAllText($file.FullName)
            Source = $null
        }
    }
}

function Get-SupplementNoticeFiles {
    <# Accepts only reviewed supplements tied to the exact crate version, commit and byte hashes. #>
    param([object] $Package, [object] $Supplement, [string] $Directory)

    $packageDirectory = Split-Path -Parent $Package.manifest_path
    $vcsFile = Get-ContainedFile -Directory $packageDirectory -Path '.cargo_vcs_info.json'
    $vcs = [IO.File]::ReadAllText($vcsFile.FullName) | ConvertFrom-Json
    if ($vcs.git.sha1 -ne $Supplement.vcs_commit) {
        throw "Supplement commit does not match $($Package.name) $($Package.version)."
    }
    if ($Supplement.PSObject.Properties.Name -contains 'verified_packaged_files') {
        foreach ($expected in $Supplement.verified_packaged_files) {
            $file = Get-ContainedFile -Directory $packageDirectory -Path $expected.path
            if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $expected.sha256) {
                throw "SDK supplement does not match packaged bytes: $($Package.name) $($expected.path)"
            }
        }
    }
    foreach ($expected in $Supplement.files) {
        $file = Get-ContainedFile -Directory $Directory -Path $expected.path
        if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $expected.sha256) {
            throw "Supplement hash mismatch: $($expected.path)"
        }
        [pscustomobject]@{
            Name = $expected.path
            Text = [IO.File]::ReadAllText($file.FullName)
            Source = $expected.source_url
        }
    }
}

Push-Location -LiteralPath $RepositoryRoot
try {
    $supplementPath = [IO.Path]::GetFullPath($SupplementManifestPath, (Get-Location).Path)
    $supplementManifest = Get-Content -LiteralPath $supplementPath -Raw | ConvertFrom-Json
    if ($supplementManifest.schema_version -ne 1) { throw 'Unsupported notice supplement schema.' }
    $supplements = @{}
    foreach ($supplement in $supplementManifest.packages) {
        $key = "$($supplement.name)|$($supplement.version)"
        if ($supplements.ContainsKey($key)) { throw "Duplicate supplement: $key" }
        $supplements[$key] = $supplement
    }
    $metadataJson = & cargo metadata --locked --offline --format-version 1 --filter-platform $targetTriple
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the cached, locked Windows dependency graph.' }
    $metadata = $metadataJson | ConvertFrom-Json
    if ($null -eq $metadata.resolve.root) { throw 'Expected a single root Cargo package.' }
    # Cargo tree retains host/target feature distinctions that metadata's unified graph loses.
    $treeLines = & cargo tree --locked --offline --target $targetTriple --edges normal,build --prefix none --format '{p}'
    if ($LASTEXITCODE -ne 0) { throw 'Could not select the locked Windows normal/build dependency graph.' }
    $selectedPackages = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($line in $treeLines) {
        if ($line -notmatch '^(?<name>\S+) v(?<version>\S+)(?: .*)?$') {
            throw "Unexpected Cargo tree package description: $line"
        }
        [void] $selectedPackages.Add("$($Matches.name)|$($Matches.version)")
    }
    # Hash-set membership keeps graph-to-metadata matching linear in the package count.
    $packages = @($metadata.packages | Where-Object {
        $selectedPackages.Contains("$($_.name)|$($_.version)") -and $_.id -ne $metadata.resolve.root
    } | Sort-Object name, version -CaseSensitive)
    $duplicates = @($packages | Group-Object name, version | Where-Object Count -gt 1)
    if ($duplicates.Count -gt 0 -or $packages.Count -ne $selectedPackages.Count - 1) {
        throw 'Cargo tree packages could not be matched unambiguously to Cargo metadata.'
    }

    $records = [System.Collections.Generic.List[object]]::new()
    $missing = [System.Collections.Generic.List[string]]::new()
    foreach ($package in $packages) {
        $files = @(Get-PackageNoticeFiles -Package $package)
        $key = "$($package.name)|$($package.version)"
        if ($supplements.ContainsKey($key)) {
            $files += @(Get-SupplementNoticeFiles -Package $package -Supplement $supplements[$key] -Directory (Split-Path -Parent $supplementPath))
        }
        if ($files.Count -eq 0 -or @($files | Where-Object { -not [string]::IsNullOrWhiteSpace($_.Text) }).Count -eq 0) {
            $missing.Add("$($package.name) $($package.version) ($($package.license))")
            continue
        }
        $records.Add([pscustomobject]@{ Package = $package; Files = $files })
    }
    if ($missing.Count -gt 0) {
        throw "Packaged license text is missing; existing output was not replaced:`n$($missing -join "`n")"
    }

    # Git may check Cargo.lock out with CRLF on Windows; fingerprint the same text on every host.
    $lockText = [IO.File]::ReadAllText((Join-Path (Get-Location).Path 'Cargo.lock')).Replace("`r`n", "`n")
    $lockBytes = [Text.UTF8Encoding]::new($false).GetBytes($lockText)
    $lockHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($lockBytes)).ToLowerInvariant()
    $output = [Text.StringBuilder]::new()
    [void] $output.AppendLine(@"
SAFEBROWSE - THIRD-PARTY NOTICES

Generated by scripts/New-ThirdPartyNotices.ps1 from the locked, cached Cargo graph.
Target: $targetTriple
Cargo.lock SHA-256 (UTF-8, LF line endings): $lockHash
Dependency packages: $($records.Count)

This inventory includes normal and build dependencies, including build-time tools
that may not be linked into the distributed executable. Development-only and
other-platform dependencies are excluded. The license identifiers and full texts
below are supplied by the respective packages. Where an archive omits its license,
reviewed upstream supplements are recorded in docs/third-party/supplements.json.
This document does not reinterpret terms or replace copyright notices.

MICROSOFT EDGE WEBVIEW2

SafeBrowse uses Microsoft's separately installed Evergreen WebView2 Runtime under
Microsoft's terms. The small installer includes Microsoft's signed, unmodified
Evergreen Bootstrapper to install that runtime when needed. Runtime updates and
the runtime's own third-party notices are managed by Microsoft.
Runtime information: https://developer.microsoft.com/microsoft-edge/webview2/
Distribution: https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution

The WebView2 SDK's loader is separate from the Runtime. Its Microsoft SDK license
and notice are included with webview2-com-sys below, alongside the Rust bindings'
MIT license. The supplied loader bytes match Microsoft.Web.WebView2 1.0.3650.58.

RUST DEPENDENCIES
"@)
    foreach ($record in $records) {
        $package = $record.Package
        [void] $output.AppendLine("`n========================================================================")
        [void] $output.AppendLine("$($package.name) $($package.version)")
        [void] $output.AppendLine("Declared license: $($package.license)")
        [void] $output.AppendLine("Package and source: https://crates.io/crates/$($package.name)/$($package.version)")
        if (-not [string]::IsNullOrWhiteSpace($package.repository)) {
            [void] $output.AppendLine("Repository: $($package.repository)")
        }
        if (@($package.authors).Count -gt 0) {
            [void] $output.AppendLine("Package authors: $($package.authors -join '; ')")
        }
        foreach ($file in $record.Files) {
            [void] $output.AppendLine("`n--- $($file.Name) ---")
            if ($null -ne $file.Source) { [void] $output.AppendLine("Upstream supplement: $($file.Source)") }
            [void] $output.AppendLine()
            [void] $output.AppendLine($file.Text)
        }
    }
    $outputAbsolutePath = [IO.Path]::GetFullPath($OutputPath, (Get-Location).Path)
    $generatedText = $output.ToString().Replace("`r`n", "`n")
    if ($Check) {
        if (-not [IO.File]::Exists($outputAbsolutePath) -or [IO.File]::ReadAllText($outputAbsolutePath) -cne $generatedText) {
            throw 'Third-party notices are missing or stale. Run scripts/New-ThirdPartyNotices.ps1 and review the changes.'
        }
        Write-Host "Verified notices for $($records.Count) dependency packages: $outputAbsolutePath"
        return
    }
    [void] [IO.Directory]::CreateDirectory((Split-Path -Parent $outputAbsolutePath))
    [IO.File]::WriteAllText($outputAbsolutePath, $generatedText, [Text.UTF8Encoding]::new($false))
    Write-Host "Generated notices for $($records.Count) dependency packages: $outputAbsolutePath"
} finally {
    Pop-Location
}
