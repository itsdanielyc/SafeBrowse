# SafeBrowse usage and engineering notes

For downloads and a quick start, see the [README](../README.md). This guide describes current behavior, advanced options and implementation limits.

## What works

The [unsigned small installer](INSTALLER.md) supports current-user installation, WebView2 setup and optional data removal. It compiles locally and has passed isolated installer integration tests; the documented clean-machine and visual checks remain. Published downloads are linked from the [README](../README.md).

- Separate Win32 desktop, with a **Back to desktop** button to leave the session and a companion taskbar entry to return to it.
- A fresh desktop identity and authenticated supervisor-to-worker startup for each isolated session. Direct worker launches are rejected.
- A taskbar anchored to the bottom of the isolated desktop, including when the browser is restored or moved. Its language button opens the installed keyboard layouts.
- Capture exclusion requested and verified before the browser window becomes visible. Launch fails if Windows cannot apply it.
- Explicit, session-only recording override for debugging, with a blocking red warning and a persistent red indicator.
- Independent web tabs with navigation history, page titles, address updates, bookmarks, and an on-screen keyboard.
- Downloads with an app-owned confirmation prompt and saved Ask / Disabled / Always allowed policy.
- Optional Print toolbar and `Ctrl+P` controls for the active website tab, disabled by default in Settings.
- System typography and bundled UI assets: the application shell does not fetch fonts or icons from third parties.
- Temporary WebView2 profiles by default, with an optional persistent profile. Bundled controls and web pages use separate browser contexts.
- Website WebViews have no messaging bridge to the application shell. Native engine events report focus, navigation, and page metadata.
- Settings provide an ask/allow/block popup policy and saved, per-site permission rules. Allowed popups open as browser tabs and retain their opener relationship.
- Website tabs and popups explicitly require WebView2 SmartScreen reputation checking and verify the native setting before navigation. Windows policy can still override that setting.
- Administrator and high-integrity launches are refused before creating a session or changing the clipboard, desktop, or profile.
- Chromium's default sandbox and site isolation remain enabled. SafeBrowse does not attempt to spoof browser fingerprints or promise to bypass anti-bot systems.

Direct file access remains unavailable. Device and web API permission requests from cross-origin frames are denied; popups use the initiating frame's exact origin. Sites that require those features, external applications, or unsupported editable fields may not work fully.

The on-screen keyboard follows the selected Windows layout, including local letters and Shift punctuation. It inserts dead keys as spacing accents and does not synthesize AltGr or IME composition; use the physical keyboard for those. Rich-text input while the editor is unfocused preserves focus and supports insertion, deletion, and line breaks, but those edits do not participate in Chromium's native undo history.

Open the keyboard with the toolbar's keyboard button. Choose **Detach**, drag its header to reposition it, then choose **Attach** to return it below the page. Detaching reuses the same keyboard document, preserving its selected layout, shuffled letters and Shift/Caps state. **Close** hides the keyboard; the toolbar button opens it again. The floating window uses the session's capture policy. Moving or shuffling keys can disrupt fixed-coordinate assumptions, but does not establish protection against keyloggers, malicious pages or a compromised browser.

### Downloads

Downloads default to **Ask every time**. In Settings, choose **Ask every time**, **Disabled**, or **Always allowed**. With Ask selected, SafeBrowse shows the filename, page origin, file address and available size before you choose **Download** or **Cancel**. Disabled requests show a notice pointing to Settings; Always allowed approves subsequent supported downloads without this prompt. These are browser-wide preferences, separate from per-site device permissions.

WebView2 handles the transfer after the native download deferral is approved. Each file receives a sanitized name inside a fresh `<Windows Downloads>\SafeBrowse\<UUID>` folder, avoiding ordinary filename collisions and overwrites. Files are never opened automatically and remain after the browsing session ends. Runtime security checks remain in place; SafeBrowse does not resume blocked or interrupted transfers.

Unanswered requests are cancelled when their page navigates or closes, when you leave the browsing desktop, or when you change the download policy. Approved transfers can continue while you return to Windows, but are cancelled on page navigation, tab closure, app shutdown, or selecting Disabled. Removal of cancelled or partial files is best effort and can leave remnants. The prompt's page origin describes the top-level page, not a verified initiating iframe.

### Printing

The **Printing** switch is directly below Downloads in Settings and defaults to **Disabled**. Enable it to use SafeBrowse's toolbar Print button and `Ctrl+P`; attempts while disabled explain how to enable these controls in Settings.

SafeBrowse also installs an early script that suppresses ordinary website `window.print()` calls and shows a dismissible message directing you to the toolbar or `Ctrl+P`. Website calls remain suppressed even when host printing is enabled. This script does not expose a native messaging bridge, call the original print function, or change focus in your form.

This is best-effort suppression, **not a complete printing block**. WebView2 has no supported global printing gate in this integration. The native probe found that an opener can retain a popup's original print function before the popup's initialization script runs; the probe inspected that reference without invoking it. Built-in browser/PDF printing paths are also separate from page JavaScript. Do not rely on the script to constrain a hostile website. Microsoft documents the [document-created script contract](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2#addscripttoexecuteondocumentcreated).

Select the website tab you want to print, then choose **Print** in the browser toolbar or press `Ctrl+P`. Choose an installed printer and confirm in the Windows print dialog, or choose **Cancel** to abandon the request. If **Microsoft Print to PDF** is installed, select it to save a PDF and choose its destination in the Windows save dialog. SafeBrowse opens the chooser; it does not silently submit print jobs or save PDFs. See [Microsoft's WebView2 printing documentation](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/print).

On the tested Windows 11 system, the native chooser provides printer, orientation and page-range controls but reports that the app does not support print preview. SafeBrowse currently has no separate page preview.

Printer, spooler and saved-file data are outside temporary-profile cleanup. Capture exclusion is not guaranteed for native print, printer-driver or save dialogs. Private-desktop compatibility and capture behavior need verification on the Windows/runtime/printer combination being used; current checks are recorded in [VALIDATION.md](VALIDATION.md).

## Build and run

Use Windows 11 or Windows 10 version 2004 or newer, the Rust MSVC toolchain pinned in [rust-toolchain.toml](../rust-toolchain.toml), Microsoft C++ Build Tools with the Windows SDK, and a current [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/). Run SafeBrowse as a normal user. The runtime must be at least `151.0.4129.107`, the version exercised by the recorded native checks. This is a tested support floor, not a guarantee of current security patches; keep Evergreen updates enabled.

```powershell
cargo build --release --locked
.\target\release\safebrowse.exe
```

The normal launch creates a fresh `WinSta0\SafeBrowseDesktop_<UUID>` session. The Back to desktop button returns to Windows. Click the existing SafeBrowse taskbar entry to return to the session. Closing the browser ends it and restores the Windows desktop. Normal launches share a session lock, including windowed mode. Any second launch now stops with a message to use that existing taskbar control or close the session first; it does not switch to a desktop or window found by a predictable name. New launch options apply after the session closes.

The supervisor retains its session desktop handle and authorizes one worker through a private inherited-handle exchange before the worker touches the clipboard or browser profile. A job ties the worker's lifetime to its supervisor before worker execution begins. These controls prevent the earlier unauthenticated launch paths; they do not isolate the host from other processes running as the same Windows user. A precreated session mutex can still deny startup. See the current boundaries in the [source security review](reviews/safebrowse-security-review.md).

Use the desktop button and companion taskbar entry for switching. Choosing a language changes SafeBrowse's input windows without installing languages or changing the Windows system default. The picker keeps focus while its embedded browser control becomes active, so opening it repeatedly does not dismiss it immediately.

The isolated mode clears the **current Windows clipboard** on entry and normal exit. Windowed development mode does not clear it. Clipboard history, cloud clipboard synchronization, and copies already read by other applications are outside this operation's scope.

### Recordable development mode

```powershell
.\target\release\safebrowse.exe --windowed --allow-screen-recording
```

`--windowed` alone retains capture protection. Only `--allow-screen-recording` disables it, for that launch. The flag also passes through to the isolated worker when used without `--windowed`.

Before website content is created, a red dialog says that screen recording is allowed, that it carries a security risk, and **“If this is a production app, stop using it.”** Click **OK** to continue. A red recording indicator stays visible throughout the session. Use test data because screenshots and recordings can expose anything displayed.

### Options

| Option | Effect |
| --- | --- |
| `--help`, `-h` | Show usage without starting a session. |
| `--windowed`, `-w` | Run on the current Windows desktop for development. |
| `--persistent`, `-p` | Keep cookies and site data after exit. |
| `--url <URL>` | Open an absolute HTTP or HTTPS address; defaults to DuckDuckGo. |
| `--allow-screen-recording` | Disable capture protection after acknowledging the red warning. |

```powershell
.\target\release\safebrowse.exe --persistent --url https://example.com
```

Unknown options, missing URL arguments, unsafe URL schemes, embedded credentials, and incompatible internal options fail before startup. `--worker` requires a live authenticated launcher and cannot be used directly. Starting from an administrator shell or using **Run as administrator** is rejected; start SafeBrowse normally instead.

### Startup troubleshooting

If a standalone launch fails, a native error dialog keeps the reason visible until you click **OK**. Launches from an existing console or with redirected error output report the failure to that console instead. Internal workers never display a startup error dialog.

**Missing or unsupported WebView2 Runtime.** SafeBrowse checks the selected runtime before opening a session, rejects preview versions and versions below its tested support floor, and validates the actual version and profile directory when creating each browser view. If no runtime is found, the startup error names the missing component and provides Microsoft's download address. Other detection errors preserve the Windows error code. These checks do not authenticate runtime binaries or establish that the installed version is fully patched.

**WebView2 overrides.** Runtime-folder, profile-folder, additional-argument, channel-selection and debugger overrides in the launch environment are unsupported. Applicable WebView2 loader policies in either registry view are also rejected. The error names the setting without copying its value. Remove the override from the launch environment, or ask the computer's administrator to review the policy. SafeBrowse does not change environment variables or registry settings.

1. Open [Microsoft's WebView2 download page](https://developer.microsoft.com/microsoft-edge/webview2/#download-section).
2. Choose the **Evergreen Bootstrapper** for an online installation, or the **Evergreen Standalone Installer** for an offline installation. Choose the standalone architecture for your device; the current SafeBrowse executable is built for Windows x64.
3. Install the runtime, then open SafeBrowse normally. The small SafeBrowse installer can install a missing or older runtime when online. Direct executable downloads require installing it separately.

The WebView2 Runtime is separate from the Microsoft Edge browser. See [Microsoft's runtime distribution guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution).

**Administrator privileges / UAC.** SafeBrowse refuses an elevated process token, rather than assuming every launch failure means UAC is disabled. First close the administrator terminal and open SafeBrowse normally, without **Run as administrator**; also check that the shortcut's compatibility settings do not force administrator mode.

If ordinary double-click launches still run elevated because UAC was disabled, use a standard Windows account or restore **User Account Control: Run all administrators in Admin Approval Mode**. On a computer you administer, open **PowerShell or Command Prompt as administrator** and run:

```powershell
reg.exe add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System" /v EnableLUA /t REG_DWORD /d 1 /f
```

**Restart Windows after the command succeeds.** Then launch SafeBrowse normally. `EnableLUA=1` enables Admin Approval Mode; it does not reset every UAC policy or choose the notification level. On a managed computer, ask the administrator to apply the policy. SafeBrowse never executes this command or changes UAC itself. See [Microsoft's UAC configuration guidance](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/settings-and-configuration).

## Storage and privacy

Temporary sessions use a unique `%TEMP%\SafeBrowse_EphemeralProfiles_v1\SafeBrowse_Session_<UUID>` directory, with an ownership marker and an exclusive session lock. Normal shutdown releases views and contexts, then retries cleanup. At the next launch, a bounded scan retries deletion of marked, inactive profiles in this root. It leaves active profiles, unrecognized directories, persistent data and legacy `%TEMP%\SafeBrowse_Session_<UUID>` directories alone; junctions and symbolic links are rejected rather than followed. A cleanup failure or scan limit prevents a new session and reports the reason. Unrecognized or locked entries are skipped, so successful startup is not proof that all old data has been removed.

A crash, power loss or locked files can still leave data behind. Old unmarked profiles require manual inspection after all relevant sessions have closed. File deletion is not forensic secure erasure; the app does not erase paging files, crash dumps, backups, or remote website records.

Persistent browser data uses `%LOCALAPPDATA%\SafeBrowse\SafeBrowse\data\Profile_Persistent`. Bookmarks and permission preferences use `bookmarks.json` and `permissions.json` in `%APPDATA%\SafeBrowse\SafeBrowse\config` and persist in both modes. The download policy and printing-controls preference are also saved in `permissions.json`. These directories are separate from ordinary Edge/Chrome profiles; they are not a separate Windows user account, encrypted vault, or per-site identity. All web tabs in one session share that session's browser profile.

Microsoft documents the cookies, cached resources, and other data stored in [WebView2 user data folders](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder).

## Website permissions and browser engine

Popups default to **Ask every time**. In Settings, choose **Always allow** or **Always block**, or add exceptions for individual sites. Other supported capabilities ask when requested. The application prompt offers **Allow once**, **Always allow**, **Block** (saved), and **Not now** (decline this request). Windows and browser restrictions still apply to an allowed request.

Site rules match the exact HTTP(S) origin: scheme, hostname, and port. They do not grant access to subdomains or other capabilities. Settings lists saved rules and lets you change or reset them. Reset returns to the popup default or to asking for other capabilities. A changed rule applies to subsequent requests; **Reload tabs** ends access already in use and discards unsaved form input in those tabs. Requests are cancelled when the requesting page navigates or closes, or when you leave the browsing desktop.

SafeBrowse owns the application shell and permission policy. It uses Wry and the installed **Microsoft Edge WebView2 Evergreen Runtime**, not CEF or a project-maintained Chromium fork. Microsoft services the shared runtime; restart SafeBrowse to use a newly installed runtime version. See [WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution). The small installer can install a missing or older runtime when online. Direct executable downloads require installing the runtime separately.

When WebView2 announces an installed update, the title bar shows **Restart to update**. Finish the current transaction and close/reopen SafeBrowse to use it. Browser exits, failed or unresponsive page renderers, and failed frame renderers end the session and cancel outstanding requests. SafeBrowse does not automatically reload pages or repeat submissions: check the website's transaction status before retrying. The supervisor attempts to restore Windows; GPU and utility failures that WebView2 documents as automatically recovered do not interrupt the session.

Source verification and candidate packaging are described in [RELEASING.md](RELEASING.md). The workflows and scripts require a clean committed source tree for release candidates, produce checksums and build metadata, and support GitHub build attestations. They do not Authenticode-sign binaries or publish a release. Existing local executables are not retroactively verified by these additions.

Every website view and popup must support `IsReputationCheckingRequired`, accept `true`, and read back `true` before website navigation. Missing support or a failed setting aborts view creation with an error. This verifies SafeBrowse's requirement, not the effective Windows policy: Microsoft documents that a disabled system SmartScreen setting overrides the saved WebView2 value. SafeBrowse does not override Windows policy or provide a separate reputation service. See [Microsoft's SmartScreen setting documentation](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2settings.isreputationcheckingrequired).

## Verification

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
node --test tests/keyboard_dom_tests.cjs
node --test tests/website_print_guard_tests.cjs
```

The tests cover launch argument validation, worker authorization, process-integrity classification, native browser security settings, Windows argument quoting, URL handling, tab state, bookmark persistence and injection resistance, keyboard input generation, and window bounds. The default suite does not clear your clipboard or switch your active desktop. Tests that deliberately do so are ignored and require explicit opt-in. Native WebView tests require an interactive Windows session and an installed runtime. Current execution results and remaining checks are recorded in [VALIDATION.md](VALIDATION.md).

Manual release checks should include:

1. Verify protected capture and the recording warning separately on each supported Windows/runtime combination; record only disposable test content.
2. Exercise tabs, back/forward, redirects, bookmarks, keyboard focus, close, and resizing at several display scales.
3. In a disposable session, verify return to Windows, companion re-entry, worker crash recovery, and actual temporary-profile deletion after exit.

Automated tests are regression checks, not proof of security against malware or compatibility with every banking site.

For keyboard security, the current tests verify input behavior and focus; they do not establish resistance to real keyloggers. The [keyboard-resistance comparison and controlled test proposal](reviews/keyboard-resistance.md) separates OS keyboard events from page-side observation and specifies disposable-VM checks using dummy data. Safepay security parity has not been established, even when website reputation and antivirus features are excluded.

The [source security review](reviews/safebrowse-security-review.md) records concrete findings and remediations. The [Safepay comparison](reviews/safepay-research.md) maps our implementation to Bitdefender's documented capabilities and identifies unverified claims and missing protections.
