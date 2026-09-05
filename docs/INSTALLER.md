# Unsigned small Windows installer

The installer source is `installer/SafeBrowse.iss`. No GitHub account or paid service is required to build locally. Published installers link to the [project](https://github.com/itsdanielyc/SafeBrowse), [support issues](https://github.com/itsdanielyc/SafeBrowse/issues), and [releases](https://github.com/itsdanielyc/SafeBrowse/releases). The publisher display name is **SafeBrowse Contributors**, the version comes from `Cargo.toml`, and the existing owner-supplied icon is reused. Publisher metadata is not a verified digital signature.

## Current validation status

The unsigned installer compiles with Inno Setup 7.1.0. The application and maintenance code passed 157 Rust tests and six read-only release-payload checks. Eleven native installer scenarios passed using production-derived scripts, unique installation/registry/shortcut identities and mock executables confined to disposable fixtures. These tests verify setup/removal orchestration without operating on the developer's browsing data. See `VALIDATION.md` for evidence and limits.

The actual application payload is packaged with Microsoft's signature-verified bootstrapper. Actual WebView2 installation on a machine without the runtime and visual interaction with the final wizard remain manual platform checks; the fixture suite simulates prerequisite results and prompt choices.

## Installation design

- Windows x64, Windows 10 build 19041 or newer; this is the technical minimum, not a claim that every Windows version is still serviced by Microsoft.
- Current-user installation at `%LOCALAPPDATA%\Programs\SafeBrowse`, with no elevation request. Elevated setup/removal is refused so it cannot clean another account's data or launch an elevated browser.
- Start menu shortcut; desktop shortcut is optional and unchecked initially.
- Production SafeBrowse uses the Windows GUI subsystem. Double-clicking it does not create a console. Fatal GUI errors still use **SafeBrowse error** dialogs. Explicit terminal/redirected launches keep diagnostics and `--help` usable.
- Existing SafeBrowse sessions must be closed by the user. Setup/removal holds a separate maintenance mutex to prevent new sessions during changes. It does not force-close sessions or launch the app automatically after setup.
- Reinstalling the same version and upgrading preserve settings. Numeric downgrades are refused.
- The small installer contains Microsoft's signed Evergreen Bootstrapper, not the full browser runtime. The maintenance helper first checks the actual WebView2 loader and supported version. Only a missing or older runtime authorizes running the bootstrapper; policy, environment, privilege and unexpected discovery failures stop setup with their diagnostic.
- WebView2 installation requires internet. With a supported runtime already present, the prerequisite check does not need a runtime download. Installation is rechecked afterward. The bootstrapper's native wait processes UI messages but currently has no time limit; a stuck Microsoft installer must be resolved before setup can finish.
- The shared Evergreen WebView2 runtime remains installed when SafeBrowse is removed.

## Removal design

Use **Windows Settings → Apps → Installed apps → SafeBrowse → Uninstall**.

After confirmation, interactive removal asks:

> Also remove SafeBrowse settings and browsing data?

**No** is the default and keeps bookmarks, preferences, saved permissions and the optional persistent browser profile. **Yes** removes those known files and the persistent profile. **Cancel** stops before cleanup. Silent uninstall keeps user data.

Both modes reclaim verified inactive temporary profiles and remove the verified temporary ownership root when empty. Cleanup uses pinned ancestors, ownership checks and exact-handle deletion; active sessions, reparse points, unexpected file types, hard links and incomplete removal are reported. A cleanup failure aborts before deleting the installed application and helper, allowing a retry. Some requested data may already have been deleted when a later failure occurs.

Downloads are always kept. Unknown configuration files, legacy unmarked temporary directories, printed output, Windows records and backups are outside automatic removal. Only empty known configuration/data folders are removed; empty parent application-data directories may remain.

The helper has no caller-selected deletion path:

```text
safebrowse-maintenance.exe check-runtime
safebrowse-maintenance.exe cleanup
safebrowse-maintenance.exe cleanup --remove-user-data
```

It is a console subprocess captured by setup, not a user-facing browsing app. Exit codes are **0** success, **10** missing/old WebView2 for `check-runtime`, and **1** blocked configuration or other failure. Do not manually run cleanup against an account whose data you want to retain.

## Build locally

Install the Windows/Rust build prerequisites in `RELEASING.md` and [Inno Setup 7.1 or newer](https://jrsoftware.org/isdl.php). No GitHub account is needed. The builder requires the official compiler's valid **Pyrsys B.V.** signature and a valid **Microsoft Corporation** signature on the runtime bootstrapper.

Run in PowerShell 7 from the repository:

```powershell
./scripts/New-Installer.ps1
```

If the compiler is installed elsewhere:

```powershell
./scripts/New-Installer.ps1 -CompilerPath 'C:\path\to\Inno Setup 7\ISCC.exe'
```

The script downloads the small bootstrapper only if it is not already cached. An existing copy can be supplied through `-RuntimeBootstrapperPath`. It rebuilds both Rust binaries in `target/installer-app`, checks the GUI subsystem and version metadata, runs read-only smoke checks, then compiles into a fresh `target/installer/<build-id>` directory. Output includes the unsigned `SafeBrowse-Setup-<version>-x64.exe`, `SHA256SUMS.txt` and `provenance.json`.

This local script accepts an uncommitted working tree and records that status; it does not provide a signed release or independently verified build. GitHub publication and a clean committed release candidate remain separate actions. It neither includes nor builds the separate `motion` project.

For a public download, commit all intended source first and run `./scripts/New-Installer.ps1 -RequireClean`. This refuses dirty or ignored build inputs and checks that the same committed revision remains unchanged through packaging. Publish that build's installer, checksums and provenance together. The installer includes `THIRD_PARTY_NOTICES.txt`; regenerate it with `scripts/New-ThirdPartyNotices.ps1` after changing dependencies, and verify it with `-Check` before release.

## Required installer test surface

Run the repeatable offline integration suite with:

```powershell
./scripts/Test-Installer.ps1
```

It compiles mock executables and production-derived installers, exercises installation and removal under fresh identifiers, verifies Start menu shortcuts and uninstall registration, and retains results/logs under `target/installer-tests`. It never runs the real cleanup helper, the real browser or Microsoft's runtime installer. The following manual checks complement that automation.

Use a disposable Windows account or VM, not an account with valuable browsing data.

1. Install with a supported runtime while disconnected from the network. Check shortcuts, Installed apps registration, product version, console-free launch, same-version reinstall and preservation of preferences. Try an older installer and verify it refuses the downgrade.
2. On a separate disposable machine without WebView2, check online installation, offline failure and retry. Invalid WebView2 configuration must stop with its own error rather than running the bootstrapper. Test setup cancellation and a failed runtime install. Never uninstall the developer's shared runtime merely to simulate absence.
3. Exercise uninstall Cancel, No, Yes and silent mode with dummy settings/profiles and downloaded sentinels. Verify choices, full intended cleanup, preservation of downloads/runtime, active-session refusal, and failure/retry when a file is locked or a path is a junction. A helper failure must leave the application and uninstaller available for retry.

Unattended tests must pass `/NORESTART /SUPPRESSMSGBOXES`; use `/VERYSILENT` only in a disposable test account. No unattended test should operate on a real user's SafeBrowse data.

References: [WebView2 deployment](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution), [Inno Setup output capture](https://jrsoftware.org/ishelp/topic_isxfunc_execandcaptureoutput.htm), [uninstall abort semantics](https://jrsoftware.org/ishelp/topic_isxfunc_abort.htm), [compiler download verification](https://jrsoftware.org/isdl-verify.php).
