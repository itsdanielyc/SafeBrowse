# SafeBrowse error and warning text catalogue

Historical application-copy snapshot, last updated for the 4 September cleanup correction. The later installer work changes console/dialog detection and adds maintenance messages; see [INSTALLER.md](../INSTALLER.md). The source references below describe the earlier snapshot and may have shifted.

Source inspection of the current local application on 4 September 2026, updated after the normal-shutdown cleanup fix and neutral error-dialog title. This records existing text and presentation; it describes the current application rather than proposed replacement copy. No real user session was interrupted and no error was deliberately triggered for this inventory.

## How to read this catalogue

- Text in code blocks and message-table cells preserves application wording, punctuation and capitalization. Where multiple separate messages share a code block, the surrounding explanation says so.
- `{error}`, `{path}`, `{version}`, `<io-error>` and similar placeholders stand for runtime substitutions. These are not literal braces or angle brackets shown to the user. Windows error strings, operating-system language, JSON parser details, file paths, site addresses and process exit codes cannot be exhaustively written out in advance. Formatting placeholders from source are explained where relevant.
- Some errors are composed from an outer message and an underlying message. The catalogue states how they join. Text can also be truncated by the native dialog or visually shortened by layout.
- Some entries are defensive paths requiring malformed application messages or exceptional platform failures; they are marked separately from normal user mistakes.
- Website-supplied content, JavaScript alerts, WebView2's built-in network/certificate/SmartScreen pages, Windows crash dialogs, native printing/save dialogs and printer-driver messages do not have fixed complete wording in SafeBrowse's source. Those external screens are outside an exact application-copy inventory.
- A caught operation failure is not always a screen. Console-only diagnostics and silent cancellation paths are identified separately to avoid presenting developer logs as user-visible dialogs.

## Where a user sees messages

| Surface | Current behavior |
| --- | --- |
| Windows error dialog | For an eligible standalone launcher, errors returned to `main` use the title `SafeBrowse error`, the Windows error icon and an OS-localized OK button. This title covers startup and session-ending errors. |
| Existing terminal / redirected output | The same returned error is written to stderr instead of opening that native dialog. Internal workers never open it. |
| Browser title-bar status | Most operation errors appear as error-colored text for eight seconds. Long text can be visually shortened; the hover title contains the full message during that interval. |
| Bookmarks / Settings status | Common notices are also sent to the loaded internal page when it provides the corresponding status function. |
| Permission prompt | Failed permission-command handling can also populate the prompt's error text and re-enable its buttons. |
| Download prompt | Has an error-text function in its HTML, but the current download-command error path uses the common browser status route rather than calling that function. |
| Language picker | Selected input-language errors appear in its own error area as well as the common notice route where propagated. |
| Recording warning | A blocking bundled warning appears before browsing only when `--allow-screen-recording` was explicitly requested. A failure after OK can append the underlying error to that warning. |
| Website print notice | The injected print wrapper places guidance inside the website document. This is informational suppression guidance, not a native error dialog. |
| Runtime update indicator | `Restart to update` is an informational title-bar label, not an error screen. |

In the normal isolated-desktop mode, an engine failure usually ends the worker. Its detailed cause is written to the worker's stderr; the supervisor displays the general session-ended message. Windowed mode can display the detailed engine failure directly. Separate companion/desktop failures can take precedence over that general message.

The following sections list exact application messages, their triggers, and source references.


## Startup, runtime and session-ending messages

Source: [main.rs](../../src/main.rs), [startup_error.rs](../../src/startup_error.rs), [runtime.rs](../../src/browser/runtime.rs), [runtime/overrides.rs](../../src/browser/runtime/overrides.rs), and [cli.rs](../../src/cli.rs).

### Presentation rules

The native error dialog title is exactly:

```text
SafeBrowse error
```

It uses the Windows error icon and an OS-provided OK button (the button label is localized by Windows). This title is also used for errors returned after a session ends. The message body is the error text below, without the console prefix. A standalone launch with its own attached console requests this dialog; launches from an existing terminal or with redirected stderr write to that output instead. Help and internal-worker invocations never show this native dialog.

Console errors are formatted as:

```text
[SafeBrowse] {error}
```

Native bodies are limited to 2,000 Unicode characters, embedded NUL characters become `�`, and truncation appends two newlines plus:

```text
Further details were written to the console.
```

Placeholders in braces below are variable substitutions, not literal characters shown to users. Constant URLs and the current minimum version are expanded. Underlying `{error}` text may itself be another message in this catalogue or a Windows/WebView2/Rust error string.

### Missing runtime

Trigger: loader reports no installed/discoverable WebView2 Runtime.

```text
Microsoft Edge WebView2 Runtime was not found. SafeBrowse needs this browser component to start.

Install Microsoft's Evergreen WebView2 Runtime from:
https://developer.microsoft.com/microsoft-edge/webview2/#download-section

Choose the Evergreen Bootstrapper, or the Evergreen Standalone Installer for an offline computer. Then reopen SafeBrowse. See README.md for setup instructions.
```

### Runtime below the supported minimum

Trigger: a stable four-component version compares below `151.0.4129.107`. `{version}` is the detected version.

```text
Microsoft Edge WebView2 Runtime {version} is older than SafeBrowse's supported minimum 151.0.4129.107. Update Evergreen WebView2 and restart SafeBrowse. This minimum is the project's tested compatibility baseline, not a guarantee of current security patches.

https://developer.microsoft.com/microsoft-edge/webview2/#download-section
```

### Preview or unrecognized runtime version

Trigger: version is not four numeric components, including a preview-channel suffix.

```text
SafeBrowse requires the stable Microsoft Edge WebView2 Runtime with a valid four-part version. Preview channels and unrecognized version strings are not supported. Install or repair Evergreen WebView2 from:
https://developer.microsoft.com/microsoft-edge/webview2/#download-section
```

### Empty runtime version

Trigger: the loader succeeds but returns an empty/whitespace version.

```text
Microsoft Edge WebView2 Runtime returned an empty version, so SafeBrowse could not verify that it is usable.

Repair or reinstall the Evergreen WebView2 Runtime from:
https://developer.microsoft.com/microsoft-edge/webview2/#download-section

Then reopen SafeBrowse. See README.md for setup and troubleshooting.
```

### Runtime discovery error

Trigger: runtime discovery returns an error other than the specifically handled missing-file error. `{HRESULT}` is eight uppercase hexadecimal digits.

```text
SafeBrowse could not check Microsoft Edge WebView2 Runtime. This may indicate a damaged installation or a Windows access restriction.

Details: {error} (HRESULT 0x{HRESULT})

Repair or reinstall the Evergreen WebView2 Runtime from:
https://developer.microsoft.com/microsoft-edge/webview2/#download-section

If this computer is managed, ask your administrator to check its WebView2 policies. See README.md for setup and troubleshooting.
```

### Environment override detected

```text
SafeBrowse cannot start while the WebView2 environment override {name} is set. Remove this override from the launch environment and reopen SafeBrowse. Runtime selection, profile redirection and browser debugging overrides are unsupported. No environment settings were changed.
```

`{name}` is the first nonempty applicable variable in this order:

```text
WEBVIEW2_BROWSER_EXECUTABLE_FOLDER
WEBVIEW2_USER_DATA_FOLDER
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
WEBVIEW2_CHANNEL_SEARCH_KIND
WEBVIEW2_RELEASE_CHANNELS
WEBVIEW2_RELEASE_CHANNEL_PREFERENCE
WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER
WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER
```

### Registry loader override detected

```text
SafeBrowse cannot start with a WebView2 loader override at {location}. This configuration can change browser arguments, runtime selection or profile storage and is unsupported, including empty or malformed policy values. Ask your administrator to remove the override for SafeBrowse. No registry settings were changed.
```

`{location}` is formatted exactly as `{hive}\{key path} [{value name}] ({registry view})`, with hive `HKLM` or `HKCU` and view `64-bit view` or `32-bit view`. Key paths come from the current/legacy WebView2 policy paths; value selectors depend on the executable/application identity. Values themselves are not displayed.

### Policy or application identity cannot be inspected

Each line is a separate exact template. Registry-access details may be composed into the first template.

```text
Cannot inspect WebView2 policy at {location}: {error}
Windows error {number} opening policy key
Windows error {number} reading policy value
Cannot identify this executable for WebView2 policy: {error}
The executable has no filename for WebView2 policy checks
Cannot inspect the process application identity (Windows error {number})
Windows returned an invalid process application identity
Cannot inspect the explicit process application identity: {error}
Windows returned an empty explicit process application identity
```

### Created runtime/profile validation fails

Trigger: an actual environment cannot report its version/storage, or the intended and actual storage directories cannot be resolved or do not match. These can appear at companion/windowed initialization or as an in-session new-tab error. Initial isolated-worker failures are presented by the supervisor as the general session-ended message instead.

```text
Cannot verify the active WebView2 Runtime version: {error}
Cannot monitor the browser engine: {error}
Cannot verify the active WebView2 profile directory: {error}
Cannot read the active WebView2 profile directory: {error}
WebView2 profile verification requires absolute directories
Cannot resolve the intended WebView2 profile directory: {error}
Cannot resolve the active WebView2 profile directory: {error}
WebView2 selected a different profile directory from the one reserved by SafeBrowse. Close SafeBrowse and check WebView2 runtime policies before restarting.
```

The version-related templates earlier in this section also apply when actual environment validation detects an unsupported version.

### Another SafeBrowse session already exists

```text
SafeBrowse is already running. Use its existing taskbar control, or close that session before starting another one.
```

If the Windows mutex itself cannot be created:

```text
Could not acquire session lock: {error}
```

### Abandoned profile cleanup blocks startup

Trigger: the startup scan records cleanup failures or hits its bound. Full composition is:

```text
Earlier temporary browser data could not be fully removed. SafeBrowse has not opened a new session. Close old SafeBrowse processes and try again; see README.md for storage locations.

{cleanup failures}
```

`{cleanup failures}` is zero or more failure messages joined by a single newline. When the scan limit is reached, the code additionally appends a newline followed by:

```text
The bounded cleanup scan reached its limit; another launch can continue cleanup.
```

With no failure entries, that creates an additional blank line before the limit sentence. Root-inspection failures return their own message directly instead of the above preamble; see the storage section.

### Isolated worker ended unsuccessfully

Trigger: the supervisor successfully restores Windows, its own dock loop did not return a separate error, and the worker exit code is nonzero. An ordinary Rust-reported worker error returns code `1`; a crash or forced termination may return another code.

```text
Browser session ended with exit code {exit_code}. Windows desktop restored. SafeBrowse did not reload pages or repeat submissions. Check the website's transaction status before trying again. Temporary session data may remain; the next launch retries cleanup.
```

The isolated worker writes its detailed cause to stderr. The supervisor does not transport that detailed cause into this dialog. If desktop restoration or companion cleanup fails first, that separate error can replace this body.

### Detailed engine failure in windowed mode

The following are the five exact engine messages, each followed by the same continuation on the same line. In isolated mode these detailed messages go to worker stderr, and the supervisor uses the general body above.

```text
The browser engine stopped unexpectedly. SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.
```

```text
A page stopped unexpectedly. SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.
```

```text
A page stopped responding. SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.
```

```text
Part of a page stopped unexpectedly. SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.
```

```text
The browser reported an unexpected process failure. SafeBrowse ended the session without reloading pages or repeating submissions. Reopen SafeBrowse and check the website's transaction status before trying again.
```

Windowed-session teardown can append desktop/profile errors separated by two newlines. In isolated-worker stderr, simultaneous clipboard cleanup failure appends:

```text
Clipboard cleanup also failed: {clipboard error}
```

### Other launcher and argument errors

Each line is a separate exact template. Most require malformed command-line input or an exceptional platform failure. Private-worker invocations are console-only, even on standalone launch.

```text
Arguments must contain valid Unicode
The internal worker entry point requires authorization from a live SafeBrowse launcher. Start SafeBrowse without internal worker arguments.
Could not acquire recovery desktop: {error}
The internal --worker flag may only be provided once
--url may only be provided once
--url requires an HTTP or HTTPS URL
Unknown argument: {argument}. Use --help for usage.
--windowed and the internal --worker flag cannot be combined
--url must not contain control characters
Invalid --url: {error}
--url requires an absolute HTTP or HTTPS URL
--url must not contain embedded credentials
```

### Installed runtime update — informational, not an error

The title-bar indicator is:

```text
Restart to update
```

Its hover text is:

```text
Close SafeBrowse and reopen when you have finished to use the installed browser update.
```

This is not a modal error or an application installer. Source: [chrome.html](../../src/ui/web/chrome.html#L59).


## Browser and in-session UI message catalogue

Source snapshot: current working tree, 2026-09-04. Read-only source trace; no browser launched or tests run. Source references point to files in this repository. Braces below mark runtime substitutions; they are not literal braces displayed to users. Literal punctuation, apostrophes and ellipses are preserved.

### Display routes and scope

- **S — Session status:** kiosk.rs:903 sends the same text to the chrome status and bundled internal page (Settings/Bookmarks). The event loop routes returned errors here at kiosk.rs:1613. Chrome status expires after 8 seconds, visually truncates long messages and retains the complete text in its hover title (web/chrome.html:186). Ordinary failures generally leave the session running.
- **P — Permission prompt error:** errors from permission-prompt commands also reach the prompt error field through kiosk.rs:1128 and permission_ui.rs:150. The download HTML defines showRequestError, but the production download-command error path does not call it: download-command failures use S.
- **B — Bookmark form:** local validation and native store errors are displayed in the open Add bookmark form as well as its page status (web/bookmarks.html:132).
- **W — Recording warning:** acknowledgment failures append the exact returned error beneath the warning and reenable OK (kiosk.rs:1111); a subsequent layout failure can supersede the original error before append.
- **K — Startup/outer error:** initial construction failures leave run_kiosk_session and reach the parent error-reporting route. Later tab/popup construction failures use S, or S/P when triggered by a permission decision.
- **Native errors:** {native_error} is the exact Windows/WebView2/Wry error Display/to_string() output. Its wording, code and localization are supplied at runtime and cannot be quoted as one fixed sentence from this repository. {io_error} is std::io::Error Display; {parse_error} is url::ParseError Display.

This section covers the browser-facing UI, navigation, printing, request/download brokers, download destinations and native browser security setup. The other sections cover fatal engine messages, runtime checks, the update indicator, stores, and Windows integration. Fixed SafeBrowse text does not exhaust arbitrary website error pages or runtime-generated TLS/SmartScreen/network/print-dialog messages.

### Ordinary navigation, tab and bookmark failures

| Exact text | Trigger and route | Source |
|---|---|---|
| Enter a web address or search term | Empty/whitespace-only navigation input. S or K for initial address. | [src/browser/navigation.rs:40](../../src/browser/navigation.rs#L40) |
| Web addresses and searches must not contain control characters | Address/search contains control characters. S/K. | [src/browser/navigation.rs:70](../../src/browser/navigation.rs#L70) |
| Web addresses must not contain backslashes | URL contains backslash. S/K. | [src/browser/navigation.rs:16](../../src/browser/navigation.rs#L16) |
| Invalid web address: {parse_error} | URL parser rejects input. S/K. | [src/browser/navigation.rs:18](../../src/browser/navigation.rs#L18) |
| Only HTTP and HTTPS web addresses are supported | Explicit address has unsupported scheme or no host. No final full stop in this variant. S/K. | [src/browser/navigation.rs:20](../../src/browser/navigation.rs#L20) |
| Web addresses containing a username or password are not supported | URL includes embedded credentials. S/K. | [src/browser/navigation.rs:24](../../src/browser/navigation.rs#L24) |
| Only HTTP and HTTPS web addresses are supported. | Website navigation handler rejects destination; includes final full stop. Popup about:blank is separately allowed. S. | [src/ui/kiosk.rs:257](../../src/ui/kiosk.rs#L257) |
| Page could not be loaded | Tab title after unsuccessful navigation except an operation-cancelled result. | [src/ui/kiosk.rs:1457](../../src/ui/kiosk.rs#L1457) |
| Page could not be loaded. Check the address and your connection, then reload. | Generic unsuccessful navigation message for the active tab. S. | [src/ui/kiosk.rs:1460](../../src/ui/kiosk.rs#L1460) |
| Close a tab before opening another (limit: 24). | Opening a web/Bookmarks/Settings tab when 24 are already open. Existing special tab may be reused. 24 is current MAX_OPEN_TABS. S. | [src/ui/kiosk.rs:1035](../../src/ui/kiosk.rs#L1035),1278 |
| Close a tab before allowing another popup (limit: 24). | Approving popup when 24 tabs are open. S/P. | [src/ui/kiosk/site_requests.rs:167](../../src/ui/kiosk/site_requests.rs#L167) |
| Open a website to bookmark it. | Add bookmark on a bundled internal tab. S. | [src/ui/kiosk.rs:1288](../../src/ui/kiosk.rs#L1288) |
| Enter a name and website address. | Empty trimmed name/address on bookmark submit; native required-field validation may intercept first. B. | [src/ui/web/bookmarks.html:162](../../src/ui/web/bookmarks.html#L162) |
| Enter a valid HTTP or HTTPS website address without sign-in details. | Bookmark form address validation fails. The caught internal string Unsupported address is not displayed. B. | [src/ui/web/bookmarks.html:167](../../src/ui/web/bookmarks.html#L167) |

Bookmark-store validation/persistence errors are propagated unmodified to S/B; see storage section.

### Printing failures and suppression notice

| Exact text | Trigger and route | Source |
|---|---|---|
| SafeBrowse print controls are disabled. You can enable them in Settings. | Print/Ctrl+P while saved host-printing preference disabled. S. | [src/browser/printing.rs:10](../../src/browser/printing.rs#L10) |
| The active tab changed. Choose Print again on the page you want to print. | Queued print command names a tab that is no longer active. S. | [src/ui/kiosk.rs:136](../../src/ui/kiosk.rs#L136) |
| Open a website to print it. | Print targets internal tab; normal button is disabled there. S, defensive/stale-command guard. | [src/ui/kiosk.rs:143](../../src/ui/kiosk.rs#L143) |
| This page is not ready to print. Wait for it to open and try again. | Target content view not available. S. | [src/ui/kiosk.rs:1201](../../src/ui/kiosk.rs#L1201) |
| Cannot access this page for printing: {native_error} | Cannot obtain CoreWebView2 for Print. S. | [src/browser/printing.rs:36](../../src/browser/printing.rs#L36) |
| Printing requires a newer Microsoft Edge WebView2 Runtime. Update the runtime using the README instructions and restart SafeBrowse. ({native_error}) | Required printing interface unavailable. S. | [src/browser/printing.rs:38](../../src/browser/printing.rs#L38) |
| Cannot open the print dialog. Close any existing print dialog and try again. ({native_error}) | ShowPrintUI rejects request. S. | [src/browser/printing.rs:42](../../src/browser/printing.rs#L42) |
| Website print dialogs are suppressed. To print, use SafeBrowse’s Print button or Ctrl+P. If printing is disabled, enable it in Settings. | Guidance inside the website/iframe when installed wrapper intercepts window.print(). Not an error screen. Button: Dismiss. Accessible region: Website printing. Accessible button: Dismiss print notice. | [src/browser/printing/website_print_guard.js:4](../../src/browser/printing/website_print_guard.js#L4) |

No success message claims a print job completed. Native print/driver dialog wording is external to this source. Popup timing can expose native print before the wrapper is installed.

### Website permission and popup failures

Native request-failure messages show only when their tab is active (site_requests.rs:111); prompt-decision failures additionally use P. Saved-policy denial normally has no separate error message.

| Exact text | Trigger and route | Source |
|---|---|---|
| Update Microsoft Edge WebView2 to manage website permissions safely | Required permission-argument interface unavailable. S. | [src/browser/requests.rs:510](../../src/browser/requests.rs#L510) |
| The requesting browser document is no longer available | Native callback has no originating WebView. S. | [src/browser/requests.rs:512](../../src/browser/requests.rs#L512) |
| This website permission is not supported | Unmapped engine permission capability. S. | [src/browser/requests.rs:527](../../src/browser/requests.rs#L527) |
| This popup was blocked because its requesting frame could not be identified. Update Microsoft Edge WebView2 and try again. | Cannot obtain original popup frame metadata. S. | [src/browser/requests.rs:562](../../src/browser/requests.rs#L562) |
| The requesting website address is too long | Permission/popup source or popup destination exceeds 8192 bytes. S. | [src/browser/requests.rs:661](../../src/browser/requests.rs#L661) |
| The requesting website address is invalid | Request source/destination URL parsing fails. S. | [src/browser/requests.rs:663](../../src/browser/requests.rs#L663) |
| Only HTTP and HTTPS websites can request permissions or popups | Request source/popup destination has no supported web origin; about:blank destination is exempt. S. | [src/browser/requests.rs:665](../../src/browser/requests.rs#L665) |
| Permissions requested by an embedded website from another origin are blocked. Open that website in its own tab. | Permission origin differs from top-level page origin. S. | [src/browser/requests.rs:682](../../src/browser/requests.rs#L682) |
| Too many website requests are waiting for a decision | Already 16 pending overall or 8 for this tab. S. | [src/browser/requests.rs:613](../../src/browser/requests.rs#L613) |
| Could not apply the permission decision: {native_error} | Engine rejects native allow/deny state. S/P. | [src/browser/requests.rs:385](../../src/browser/requests.rs#L385) |
| Could not complete the website request: {native_error} | Completing native request deferral fails. S/P. | [src/browser/requests.rs:200](../../src/browser/requests.rs#L200) |
| Could not open the popup inside SafeBrowse: {native_error} | SetHandled/SetNewWindow fails. S/P. | [src/browser/requests.rs:422](../../src/browser/requests.rs#L422) |
| Cannot initialize website permission policy: {native_error} | Cannot obtain profile/interface for resetting old native decisions before first website navigation. K/W/S. | [src/browser/requests.rs:68](../../src/browser/requests.rs#L68) |
| Cannot read existing website permissions: {native_error} | Starting native asynchronous settings lookup fails. K/W/S. | [src/browser/requests.rs:97](../../src/browser/requests.rs#L97) |
| WebView2 returned no permission settings | Asynchronous callback supplies no collection. S. | [src/browser/requests.rs:78](../../src/browser/requests.rs#L78) |
| Cannot inspect existing website permissions: {native_error} | Reading native permission collection count/records fails. S. | [src/browser/requests.rs:124](../../src/browser/requests.rs#L124) |

PermissionStore errors (see storage section) propagate from saved-rule lookup, prompt remember, defaults/site rules, and reload-origin validation. Asynchronous native reset failures can also be bare {native_error}; failed reset state is retained and repeated on later navigation attempts (site_requests.rs:18,45).

### Download failures

Download broker failure events use S even if their tab is inactive (file_requests.rs:35). Canceling normally has no cancellation message.

| Exact text | Trigger and route | Source |
|---|---|---|
| Downloads are disabled. You can change this in Settings. | Request arrives while policy is Block; error styling. S. | [src/ui/kiosk/file_requests.rs:21](../../src/ui/kiosk/file_requests.rs#L21) |
| Too many download requests. Answer an existing request before downloading again. | Already 8 pending overall or 3 for this tab. S. | [src/browser/downloads.rs:415](../../src/browser/downloads.rs#L415) |
| Too many active downloads. Wait for a transfer to finish and try again. | Approving when 4 transfers active. S. | [src/browser/downloads.rs:277](../../src/browser/downloads.rs#L277) |
| The download address is too long to display safely | Native page/download URL exceeds 8192 bytes. S. | [src/browser/downloads.rs:504](../../src/browser/downloads.rs#L504) |
| The download address is invalid | Download URL parser fails. S. | [src/browser/downloads.rs:510](../../src/browser/downloads.rs#L510) |
| Only web downloads and files generated by the current website are supported. | Unsupported scheme, or blob origin differs from page origin. S. | [src/browser/downloads.rs:515](../../src/browser/downloads.rs#L515) |
| Cannot identify the download's browser tab | DownloadStarting sender absent. S. | [src/browser/downloads.rs:427](../../src/browser/downloads.rs#L427) |
| Windows did not provide a Downloads folder | User Downloads directory unavailable. S. | [src/browser/downloads/destination.rs:28](../../src/browser/downloads/destination.rs#L28) |
| The Downloads folder must be an absolute path | Resolved output root relative; defensive guard. S. | [src/browser/downloads/destination.rs:31](../../src/browser/downloads/destination.rs#L31) |
| Cannot create the download folder: {io_error} | Cannot create Downloads/SafeBrowse directory tree. S. | [src/browser/downloads/destination.rs:34](../../src/browser/downloads/destination.rs#L34) |
| Cannot reserve a fresh download destination: {io_error} | Cannot create per-transfer UUID directory. S. | [src/browser/downloads/destination.rs:37](../../src/browser/downloads/destination.rs#L37) |
| Cannot choose the download destination: {native_error} | SetResultFilePath fails. S. | [src/browser/downloads.rs:295](../../src/browser/downloads.rs#L295) |
| Cannot observe download completion: {native_error} | StateChanged registration fails. S. | [src/browser/downloads.rs:309](../../src/browser/downloads.rs#L309) |
| Cannot allow this download: {native_error} | SetHandled or clearing cancellation fails. S. | [src/browser/downloads.rs:315](../../src/browser/downloads.rs#L315) |
| Could not complete the download decision: {native_error} | Deferral completion fails. S. | [src/browser/downloads.rs:91](../../src/browser/downloads.rs#L91) |
| Cannot suppress the canceled download's dialog: {native_error} | SetHandled fails after initial denial succeeds. S. | [src/browser/downloads.rs:400](../../src/browser/downloads.rs#L400) |
| SafeBrowse could not defer this download, so it canceled the native transfer instead. ({request_error}) | Initial request cancellation fails but independent transfer cancellation succeeds. request_error is native Display. S. | [src/browser/downloads.rs:494](../../src/browser/downloads.rs#L494) |
| This tab was closed because download protection failed. SafeBrowse could not block an unapproved download. The affected tab must close. Request cancellation: {request_error}. Transfer cancellation: {operation_error}. | Both cancellation paths fail. This is the complete COMPOSED text including UI prefix. Affected tab closes; Settings is opened if it was the last tab. Both substitutions are native Display strings. S. | [src/ui/kiosk/file_requests.rs:69](../../src/ui/kiosk/file_requests.rs#L69); [src/browser/downloads.rs:497](../../src/browser/downloads.rs#L497) |
| Download did not finish: {file_name} ({detail}). Runtime security blocks are not bypassed. | Transfer interrupted or terminal-state query fails. file_name is sanitized native suggestion. detail is native Display or exactly WebView2 interruption code {number}. S. | [src/browser/downloads.rs:592](../../src/browser/downloads.rs#L592) |

Page/download HTTP(S) origin normalization also uses PermissionStore normalize_origin error strings (storage section). Best-effort incomplete-download cleanup ignores errors and adds no user message.

### Browser construction/security attachment technical templates

These are real failure paths, not ordinary control validation. They can produce K at initial construction, W when acknowledging the recording warning, or S/P when opening a later website/popup. Runtime-version/profile-verification errors are in the startup/runtime section.

| Exact template | Failure | Source |
|---|---|---|
| Cannot create {surface} view: {native_error} | Bundled WebView creation. surface is Debug name: Chrome, Taskbar, Keyboard, Internal, Warning, LanguagePicker, PermissionPrompt or DownloadPrompt. | [src/ui/kiosk.rs:196](../../src/ui/kiosk.rs#L196) |
| Cannot load {surface} controls: {native_error} | Initial bundled HTML load after monitor/validation. Same surface names. | [src/ui/kiosk.rs:206](../../src/ui/kiosk.rs#L206) |
| Cannot create SafeBrowse download request: {native_error} | Hidden native download prompt window construction at startup, not first download. | [src/ui/permission_ui.rs:60](../../src/ui/permission_ui.rs#L60); [src/ui/shell_windows.rs:66](../../src/ui/shell_windows.rs#L66) |
| Cannot create SafeBrowse website request: {native_error} | Hidden native website permission prompt window construction at startup. | [src/ui/permission_ui.rs:62](../../src/ui/permission_ui.rs#L62); [src/ui/shell_windows.rs:66](../../src/ui/shell_windows.rs#L66) |
| Cannot create browser tab: {native_error} | Website/popup WebView construction. | [src/ui/kiosk.rs:276](../../src/ui/kiosk.rs#L276) |
| Cannot monitor browser focus: {native_error} | Focus/accelerator callback registration. | [src/ui/kiosk.rs:377](../../src/ui/kiosk.rs#L377) |
| Cannot monitor page navigation: {native_error} | Source/navigation callback registration. | [src/ui/kiosk.rs:445](../../src/ui/kiosk.rs#L445) |
| Cannot monitor the browser engine: {native_error} | Engine lifecycle/update monitor attachment. | [src/browser/health.rs:123](../../src/browser/health.rs#L123) |
| Cannot read website security settings: {native_error} | Cannot read CoreWebView2.Settings. | [src/browser/security.rs:17](../../src/browser/security.rs#L17) |
| Cannot disable website access to native host bridges: {native_error} | Cannot disable web messaging/host-object access. | [src/browser/security.rs:23](../../src/browser/security.rs#L23) |
| Microsoft Edge WebView2 does not support the required SmartScreen setting; update the WebView2 Runtime: {native_error} | Required settings interface unavailable. | [src/browser/security.rs:28](../../src/browser/security.rs#L28) |
| Cannot require SmartScreen reputation checking: {native_error} | Setting/readback fails. | [src/browser/security.rs:37](../../src/browser/security.rs#L37) |
| WebView2 did not retain the required SmartScreen reputation-checking setting | Readback false. | [src/browser/security.rs:40](../../src/browser/security.rs#L40) |
| Cannot monitor website permissions: {native_error} | PermissionRequested registration fails. | [src/browser/requests.rs:287](../../src/browser/requests.rs#L287) |
| Cannot monitor website popups: {native_error} | NewWindowRequested registration fails. | [src/browser/requests.rs:314](../../src/browser/requests.rs#L314) |
| Cannot invalidate website requests on navigation: {native_error} | Top-level navigation invalidation registration fails. | [src/browser/requests.rs:329](../../src/browser/requests.rs#L329) |
| Cannot invalidate website requests on frame navigation: {native_error} | Frame navigation invalidation registration fails. | [src/browser/requests.rs:335](../../src/browser/requests.rs#L335) |
| Cannot monitor popup closure: {native_error} | WindowCloseRequested registration fails. | [src/browser/requests.rs:347](../../src/browser/requests.rs#L347) |
| Update Microsoft Edge WebView2 Runtime to manage downloads safely: {native_error} | Required download interface unavailable. | [src/browser/downloads.rs:173](../../src/browser/downloads.rs#L173) |
| Cannot monitor downloads: {native_error} | DownloadStarting registration fails. | [src/browser/downloads.rs:208](../../src/browser/downloads.rs#L208) |
| Cannot invalidate downloads on navigation: {native_error} | Cancellation-on-navigation registration fails. | [src/browser/downloads.rs:222](../../src/browser/downloads.rs#L222) |
| Cannot invalidate downloads on tab closure: {native_error} | Cancellation-on-close registration fails. | [src/browser/downloads.rs:236](../../src/browser/downloads.rs#L236) |
| Cannot resize browser controls: {native_error} | Bounds/visibility layout fails at initialization, resize, tab/shell change. | [src/ui/kiosk.rs:797](../../src/ui/kiosk.rs#L797) |
| Cannot update browser window position: {native_error} | NotifyParentWindowPositionChanged fails during host movement. | [src/ui/kiosk.rs:898](../../src/ui/kiosk.rs#L898) |

Other native errors are passed through as the entire message, **{native_error}**, without an app prefix:

- WebView show/hide, internal HTML reload, script evaluation and focus: kiosk.rs:825,829,851,933,949,959,964,999,1004,1008,1029,1210,1258,1264,1341,1365,1382.
- Native website navigation/reload and policy/bookmark UI refresh: site_requests.rs:16,54,69,199,297.
- Prompt show/hide/resize/focus/script errors: permission_ui.rs:126,129,133,143,164.
- Async permission lookup/reset HRESULTs: requests.rs:75,149,156.
- Native permission/popup metadata and deferral calls: requests.rs:507,511,519,522,523,530,557,564,568,572,573,673; popup URL read at413.
- Native download metadata/deferral calls: downloads.rs:431,436,438,444,449,459.

### Developer/state guards, separated from ordinary failures

The following command-validation errors require malformed trusted-control IPC or an app defect; normal bundled controls supply the fields/types/surfaces. They are not separate end-user error screens and website IPC is disabled. They nevertheless use S, plus P for permission-prompt errors.

| Exact text | Guard | Source |
|---|---|---|
| Invalid browser control message | Invalid JSON. | [src/ui/kiosk.rs:1089](../../src/ui/kiosk.rs#L1089) |
| Missing browser command | Missing string type. | [src/ui/kiosk.rs:1093](../../src/ui/kiosk.rs#L1093) |
| This control cannot send that keyboard command. | Disallowed source surface. | [src/ui/kiosk.rs:1095](../../src/ui/kiosk.rs#L1095) |
| Missing {key} | Missing required string: actual key is url, title, id or action. | [src/ui/kiosk.rs:1165](../../src/ui/kiosk.rs#L1165) |
| Invalid tab ID | ID missing/not representable unsigned integer. | [src/ui/kiosk.rs:1172](../../src/ui/kiosk.rs#L1172) |
| Printing is only available from the browser's Print control or Ctrl+P. | PRINT from non-Chrome source. | [src/ui/kiosk.rs:131](../../src/ui/kiosk.rs#L131) |
| Invalid permission prompt source. | Wrong RESOLVE_SITE_REQUEST source. | [src/ui/kiosk/site_requests.rs:227](../../src/ui/kiosk/site_requests.rs#L227) |
| Missing request ID | Missing unsigned request ID. | [src/ui/kiosk/site_requests.rs:232](../../src/ui/kiosk/site_requests.rs#L232) |
| Invalid permission decision | Unknown/malformed decision. | [src/ui/kiosk/site_requests.rs:242](../../src/ui/kiosk/site_requests.rs#L242),308 |
| Choose allow or block for this request. | Prompt responds Ask. | [src/ui/kiosk/site_requests.rs:244](../../src/ui/kiosk/site_requests.rs#L244) |
| Missing decision duration | Missing Boolean remember. | [src/ui/kiosk/site_requests.rs:249](../../src/ui/kiosk/site_requests.rs#L249) |
| Permission settings are only available in Settings. | Wrong source/current tab for settings command. | [src/ui/kiosk/site_requests.rs:273](../../src/ui/kiosk/site_requests.rs#L273) |
| Missing website address | Missing string origin. | [src/ui/kiosk/site_requests.rs:280](../../src/ui/kiosk/site_requests.rs#L280),316 |
| Unknown site permission | Capability cannot deserialize. | [src/ui/kiosk/site_requests.rs:320](../../src/ui/kiosk/site_requests.rs#L320) |
| Invalid download confirmation source. | Wrong RESOLVE_DOWNLOAD source. | [src/ui/kiosk/file_requests.rs:93](../../src/ui/kiosk/file_requests.rs#L93) |
| Missing download ID | Missing unsigned request ID. | [src/ui/kiosk/file_requests.rs:98](../../src/ui/kiosk/file_requests.rs#L98) |
| Missing download decision | Missing Boolean allow. | [src/ui/kiosk/file_requests.rs:102](../../src/ui/kiosk/file_requests.rs#L102) |
| Download and printing preferences can only be changed in Settings. | Wrong source/current tab. | [src/ui/kiosk/file_requests.rs:131](../../src/ui/kiosk/file_requests.rs#L131) |
| Invalid download policy | Malformed policy decision. | [src/ui/kiosk/file_requests.rs:139](../../src/ui/kiosk/file_requests.rs#L139) |
| Invalid printing preference | Missing Boolean enabled. | [src/ui/kiosk/file_requests.rs:153](../../src/ui/kiosk/file_requests.rs#L153) |

Rare broker expiry/state guards with production propagation paths include:

| Exact text | Guard | Source |
|---|---|---|
| This website request has expired | Broker lookup/take finds no request. Normal stale prompt replies are checked and ignored before this. | [src/browser/requests.rs:396](../../src/browser/requests.rs#L396),451 |
| This download request expired or was already answered | Download broker resolve finds no request. Normal stale prompt replies are checked and ignored first. | [src/browser/downloads.rs:268](../../src/browser/downloads.rs#L268) |
| The requesting browser document is no longer available | Missing native callback sender; also catalogued above. | [src/browser/requests.rs:512](../../src/browser/requests.rs#L512) |

Not enumerated as user cases: impossible-with-current-callers construction/profile absence checks, exhausted u64 counters, double-completion assertions, mismatched request-kind/environment invariants, and cfg(test)/unused BrowserController messages. These are code invariants rather than demonstrated user-facing workflows.

### Blocking recording warning — warning, not failure

Appears whenever capture_allowed is true, before website creation/navigation. Exact visible text from [src/ui/web/capture-warning.html:23-27](../../src/ui/web/capture-warning.html#L23) and button at28. Block/paragraph boundaries are preserved below; natural line wrapping depends on window size.

<pre>SafeBrowse · Testing mode
Screen recording is allowed
Screenshots and screen recording are enabled for this session. Anyone with capture access could record passwords, payment details, and other sensitive information displayed here.
This carries a security risk.
If this is a production app, stop using it.
Continue only with test accounts and non-sensitive information.
OK</pre>

### Other warnings/guidance, not errors

| Exact text | Where/when | Source |
|---|---|---|
| Recording allowed | Chrome badge with capture enabled. | [src/ui/web/chrome.html:97](../../src/ui/web/chrome.html#L97) |
| Security risk: screenshots and screen recording are allowed in this session. | Capture badge hover title. | [src/ui/web/chrome.html:100](../../src/ui/web/chrome.html#L100) |
| Recording allowed · Testing only | Taskbar capture-mode status. | [src/ui/web/taskbar.html:37](../../src/ui/web/taskbar.html#L37) |
| This connection is not encrypted. | HTTP indicator hover title; indicator text HTTP. | [src/ui/web/chrome.html:161](../../src/ui/web/chrome.html#L161) |
| HTTPS connection. Always check the website address. | HTTPS indicator hover title; indicator text HTTPS. | [src/ui/web/chrome.html:161](../../src/ui/web/chrome.html#L161) |
| Only download files you trust. Files are saved in Downloads / SafeBrowse, remain after the session ends, and are never opened automatically. | Download confirmation guidance. | [src/ui/web/download-prompt.html:20](../../src/ui/web/download-prompt.html#L20) |
| Check the address before saving. A bookmark does not verify a website. | Add bookmark form. | [src/ui/web/bookmarks.html:50](../../src/ui/web/bookmarks.html#L50) |
| Security risk: screenshots and recordings can contain sensitive information. If this is a production app, stop using it. | Settings capture-enabled description. | [src/ui/web/settings.html:83](../../src/ui/web/settings.html#L83) |
| SafeBrowse requests exclusion from supported Windows screen-capture tools. This is not a guarantee against every capture method. | Settings capture-excluded description. | [src/ui/web/settings.html:83](../../src/ui/web/settings.html#L83) |
| This session runs on your normal Windows desktop without desktop isolation. | Settings nonisolated session description. | [src/ui/web/settings.html:80](../../src/ui/web/settings.html#L80) |
| Choose whether websites may save files. Downloaded files remain on your computer after the session ends. | Settings downloads description. | [src/ui/web/settings.html:53](../../src/ui/web/settings.html#L53) |
| Enable SafeBrowse’s Print button and Ctrl+P. Website print requests are suppressed where possible; this is not a complete printing block. Printed output is outside session cleanup. | Settings printing description. | [src/ui/web/settings.html:54](../../src/ui/web/settings.html#L54) |
| Saved choices apply to the exact website origin, including its port, and remain between sessions. Reset removes an exception. After revoking access, use Reload tabs or close the website to end access already in use. Reloading discards unsaved form input. | Settings permission-rule guidance. | [src/ui/web/settings.html:66](../../src/ui/web/settings.html#L66) |
| To apply changed permissions to active pages from {origin}, reload their tabs. Unsaved form input will be discarded. | Settings after changing/resetting an existing rule; origin is saved canonical origin. | [src/ui/web/settings.html:103](../../src/ui/web/settings.html#L103) |
| Experimental software. Security protections are limited by Windows and the browser runtime. | Settings About text. | [src/ui/web/settings.html:73](../../src/ui/web/settings.html#L73) |

### Ordinary success/progress/empty states, not errors

| Exact text | Trigger/substitution | Source |
|---|---|---|
| Bookmark saved. | Successful add. | [src/ui/kiosk.rs:1293](../../src/ui/kiosk.rs#L1293),1300 |
| Bookmark removed. | Successful removal. | [src/ui/kiosk.rs:1305](../../src/ui/kiosk.rs#L1305) |
| Preferences saved. | Successful download/printing preference update. | [src/ui/kiosk/file_requests.rs:160](../../src/ui/kiosk/file_requests.rs#L160) |
| Permission settings saved. Reload or close a website to end access already in use. | Successful default/site permission update. | [src/ui/kiosk/site_requests.rs:331](../../src/ui/kiosk/site_requests.rs#L331) |
| Reloaded {reloaded} tab(s) for {origin}. | Number of matching open tabs whose Reload call succeeds; canonical origin. | [src/ui/kiosk/site_requests.rs:301](../../src/ui/kiosk/site_requests.rs#L301) |
| Downloading {file_name}… | Transfer approved; filename sanitized. | [src/ui/kiosk/file_requests.rs:30](../../src/ui/kiosk/file_requests.rs#L30) |
| Download saved to {path} | Engine reports COMPLETED; absolute destination is Downloads/SafeBrowse/{uuid}/{sanitized_file_name}. | [src/ui/kiosk/file_requests.rs:34](../../src/ui/kiosk/file_requests.rs#L34) |
| Saving… | Bookmark form awaiting native response. | [src/ui/web/bookmarks.html:172](../../src/ui/web/bookmarks.html#L172) |
| No bookmarks yet | Empty bookmarks with no search filter. | [src/ui/web/bookmarks.html:128](../../src/ui/web/bookmarks.html#L128) |
| Add a website to get started. | Empty bookmark description. | [src/ui/web/bookmarks.html:129](../../src/ui/web/bookmarks.html#L129) |
| No matching bookmarks | Search excludes all bookmarks. | [src/ui/web/bookmarks.html:128](../../src/ui/web/bookmarks.html#L128) |
| Try a different name or website address. | Empty search-result guidance. | [src/ui/web/bookmarks.html:129](../../src/ui/web/bookmarks.html#L129) |
| No saved site permissions. | Empty permission list. | [src/ui/web/settings.html:144](../../src/ui/web/settings.html#L144) |
| Size unknown | Download has no known byte count. | [src/ui/web/download-prompt.html:32](../../src/ui/web/download-prompt.html#L32) |
| Size: {locale_formatted_total_bytes} bytes | Known size; JavaScript Number(...).toLocaleString determines digit grouping. | [src/ui/web/download-prompt.html:32](../../src/ui/web/download-prompt.html#L32) |
| Loading… | Download/printing settings wait for native snapshot. | [src/ui/web/settings.html:53](../../src/ui/web/settings.html#L53),54 |

The normal permission approval prompts are not errors. Their exact headings and descriptions are paired here; each is a separate text element (not a slash-separated app string). Native origin is displayed separately; popup destination text is Destination: {target_url}. Source: [src/ui/web/permission-prompt.html:26-45](../../src/ui/web/permission-prompt.html#L26).

| Heading | Description |
|---|---|
| Open a pop-up? | This website wants to open another SafeBrowse tab. |
| Allow camera access? | This website wants to use your camera. |
| Allow microphone access? | This website wants to use your microphone. |
| Share your location? | This website wants to know your location. |
| Allow notifications? | This website wants to show notifications. |
| Allow clipboard access? | This website wants to read text or other content from your clipboard. |
| Allow motion sensors? | This website wants to read motion or orientation sensors. |
| Allow access to local fonts? | This website wants to see fonts installed on your computer. |
| Allow MIDI device access? | This website wants to exchange system messages with connected MIDI devices. |
| Allow window management? | This website wants information about your screens and to arrange windows. |
| Allow automatic playback? | This website wants to play media without waiting for you to start it. |

Buttons: Block; Allow once; Always allow; Not now. Download prompt heading: Download this file?; fields: {file_name}, Page: {origin}, File source: {url}, size above; buttons Cancel and Download (web/download-prompt.html:18,22,29-32). The unused generic permission fallback is not counted as a normal case.

### Silent paths and limitations

- No cfg(test) fixture strings or unused BrowserController wrappers included.
- Console-only messages such as Cannot update controls, floating language-bar suppression failure, and PrintScreen registration failure are not in-app error screens.
- Saved-policy permission/popup denial, ordinary canceled download, inactive-tab request failure, unknown UI command, stale nonmatching prompt reply and best-effort failed UI script update can produce no visible message.
- HTML input constraints can display runtime/localized validation bubbles before SafeBrowse submit handlers execute. Their exact wording, like native print/TLS/network/reputation UI and website-authored errors, is not controlled by these files.
- The runtime, storage and Windows sections complete the propagated-message catalogue for the same UI routes.


## Storage error-message catalogue

Read-only source catalogue, 2026-09-04. Only application profile storage, bookmark storage, and permission persistence/origin validation are covered here. No application code or tests were changed or executed for this catalogue. Source locations refer to the current working tree, not the earlier review snapshot.

In the exact templates below, `<io-error>` is the runtime's `std::io::Error` display text, `<json-error>` is the actual Serde JSON error, and `<url-parse-error>` is the URL library's error. Their content can depend on input, Windows language, and runtime; the application does not substitute a fixed message. `<root>` and `<session-directory>` use Rust `Path::display()`. `\n` in a composition template denotes an actual newline, not a literal backslash followed by n. Leaf messages listed in tables have no application-inserted newline.

### Where these errors appear

- **Startup (S):** bookmark and permission initialization and browser-profile creation fail session setup. With `--windowed`, the actual error reaches the launcher's normal native/console error reporter. In the ordinary isolated mode, these happen inside the worker: the exact error is written to worker stderr, and the launcher presents its generic `Browser session ended with exit code <exit-code>...` message instead. They are not individual error pages inside a usable session. Source: [kiosk.rs:684](../../src/ui/kiosk.rs#L684), [kiosk.rs:1511](../../src/ui/kiosk.rs#L1511), [main.rs:201](../../src/main.rs#L201).
- **Prelaunch recovery (R):** abandoned-profile scanning runs in the launcher before a new session. Errors returned directly by that scan reach normal native/console presentation, even for isolated browsing. Per-profile cleanup failures are inserted into the launcher's `Earlier temporary browser data could not be fully removed...` message. The startup/runtime section contains the entire preamble and limit suffix. Source: [main.rs:148](../../src/main.rs#L148).
- **In-session notice (N):** bookmark writes and Settings permission writes propagate to the event handler. It writes `[SafeBrowse] <error>` to stderr and passes the exact unprefixed error to the chrome status notice and internal page's status functions. On the Bookmarks page, an open add form can also show it inline. Permission-prompt errors additionally call `permission_ui.show_error`, so remembered-grant persistence failures can appear inline in that prompt. These are notices, not native startup dialogs. Source: [kiosk.rs:903](../../src/ui/kiosk.rs#L903), [kiosk.rs:1130](../../src/ui/kiosk.rs#L1130), [kiosk.rs:1615](../../src/ui/kiosk.rs#L1615).
- **Shutdown (X):** explicit temporary-profile cleanup can return its error alone or after terminal browser/desktop errors, separated by `\n\n`. It then follows the same windowed-direct / isolated-worker-generic distinction as S. The supervisor's separate default-desktop dock profile is created and cleaned in the launcher process, so its failures can reach native/console presentation directly. Source: [kiosk.rs:1620](../../src/ui/kiosk.rs#L1620), [dock.rs:235](../../src/desktop/dock.rs#L235), [dock.rs:349](../../src/desktop/dock.rs#L349).

Native/console formatting is owned by [startup_error.rs:40](../../src/startup_error.rs#L40): stderr prepends `[SafeBrowse] `, native title is `SafeBrowse error`, and native body applies the common length/NUL handling documented in the startup/runtime section. A console-only launch can therefore have no native error screen. UI notice delivery itself is best effort if the view is unavailable.

### Profiles: outer messages and composition

The owned temporary root is `std::env::temp_dir()\SafeBrowse_EphemeralProfiles_v1`; each session has a UUID-bearing subdirectory. Paths are dynamic and are included only where the template says so.

| Exact error/template | Trigger and route | Source |
| --- | --- | --- |
| `Failed to resolve app data directory` | Persistent profile requested, but application data directory cannot be resolved. S. | [profile.rs:63](../../src/browser/profile.rs#L63) |
| `Failed to create persistent profile directory: <io-error>` | Persistent profile directory creation fails. S. | [profile.rs:68](../../src/browser/profile.rs#L68) |
| `Cannot create temporary browser profile at <root>: <storage-error>` | Creating/pinning the owned root or new session, validating root ownership, creating/writing/syncing a marker, or obtaining handles fails. `<storage-error>` can be a fixed leaf below or an OS I/O error. S; directly from launcher for dock-profile creation. | [profile.rs:80](../../src/browser/profile.rs#L80) |
| `Profile cleanup state was poisoned` | Explicit cleanup cannot lock the profile's cleanup mutex after poisoning. X; defensive exceptional case. | [profile.rs:119](../../src/browser/profile.rs#L119) |
| `Temporary browser data remains at <session-directory>: <storage-error>` | Active-session cleanup fails, after retries for transient Windows locks; or an abandoned, successfully leased profile cannot be deleted. X or one R failure entry. | [profile.rs:135](../../src/browser/profile.rs#L135), [profile.rs:217](../../src/browser/profile.rs#L217) |
| `Cannot inspect temporary profile storage at <root>: <storage-error>` | Opening/pinning/validating the existing recovery root fails. R, returned directly; no `Earlier temporary...` wrapper. | [profile.rs:177](../../src/browser/profile.rs#L177) |
| `<io-error>` | Initial `read_dir` of a validated recovery root fails. R, returned directly with no storage-specific prefix. | [profile.rs:186](../../src/browser/profile.rs#L186) |
| `<io-error>` | One `read_dir` entry fails while scanning the recovery root. Added as one R failure entry under the launcher's recovery preamble. | [profile.rs:195](../../src/browser/profile.rs#L195) |

Recovery failure entries are joined with one `\n`; the launcher's preamble is followed by `\n\n`, those joined entries, and an optional limit suffix. The suffix can appear even with no individual failure entry. It is not accurate to show only the leaf error as the complete recovery dialog.

On ordinary kiosk shutdown, the full error is `join("\n\n", [terminal-error if any, desktop-return-error if any, profile-cleanup-error if any])`. The worker can further append `\n\nClipboard cleanup also failed: <clipboard-error>` when session and clipboard cleanup both fail ([main.rs:129](../../src/main.rs#L129)). On dock shutdown, a dock error and profile error combine as `<dock-error>\n\n<profile-cleanup-error>`; a single error is returned unchanged ([dock.rs:350](../../src/desktop/dock.rs#L350)).

If setup unwinds before explicit cleanup, the profile destructor can emit `SafeBrowse profile cleanup: <profile-cleanup-error>` to stderr ([profile.rs:158](../../src/browser/profile.rs#L158)). That destructor message is **console-only** and is not itself added to the native error body.

#### Fixed temporary-storage leaf errors

These strings are not standalone dialogs. They replace `<storage-error>` in the outer templates where the failing operation is reached.

| Exact leaf text | Trigger / applicable outer message | Source |
| --- | --- | --- |
| `Temporary profile root has no valid ownership marker` | Opened root marker content differs from the required versioned bytes. Create or inspect outer message. | [storage.rs:114](../../src/browser/profile/storage.rs#L114) |
| `Profile storage requires an absolute local drive path` | Root parent path has no local disk or verbatim disk prefix; UNC and other prefix kinds are rejected. Create or inspect. | [storage.rs:218](../../src/browser/profile/storage.rs#L218) |
| `Profile storage must not use drive-relative paths` | Disk prefix is not followed by a root-directory component. Create or inspect. | [storage.rs:224](../../src/browser/profile/storage.rs#L224) |
| `Profile storage cannot contain parent traversal` | A later path component is not a normal component. Create or inspect. | [storage.rs:232](../../src/browser/profile/storage.rs#L232) |
| `Ownership marker must be a regular, singly linked file` | Marker is a directory or has link count other than one. Root-marker checks can surface through create/inspect; session-marker checks during recovery reopen are skipped without a screen. | [storage.rs:271](../../src/browser/profile/storage.rs#L271) |
| `Ownership marker is too large` | Read marker exceeds 128 bytes. Same root/session presentation distinction. | [storage.rs:277](../../src/browser/profile/storage.rs#L277) |
| `Refusing a reparse point in temporary profile storage` | An opened pinned object/marker has the Windows reparse-point attribute. Create, inspect, or cleanup, depending on object. | [storage.rs:298](../../src/browser/profile/storage.rs#L298) |
| `Expected a temporary profile directory` | A handle that must represent a root, ancestor, or session directory is not a directory. Create or inspect; recovery reopen errors are skipped. | [storage.rs:306](../../src/browser/profile/storage.rs#L306) |
| `Temporary profile cleanup reached its work limit` | Cleanup deadline, 50,000 visited-node budget, or depth greater than 64 is reached. Cleanup outer message. Recovery may additionally append its scan-limit suffix. | [storage.rs:343](../../src/browser/profile/storage.rs#L343) |
| `Refusing a multiply linked temporary profile file` | A non-directory child to delete has link count other than one. Cleanup outer message. | [storage.rs:386](../../src/browser/profile/storage.rs#L386) |

Other ordinary storage failures (open/create/read/write/sync/enumerate/delete and native handle information/deletion APIs) use raw `<io-error>` within the applicable outer message. A missing recovery root is successful and produces no error.

Do **not** count these internal strings as additional current error-screen cases: `Profile root disappeared`, `Session path does not match the owned root`, and `Session ownership marker does not match its directory` are exclusive to `SessionLease::reopen`, whose errors are swallowed as skipped entries ([profile.rs:207](../../src/browser/profile.rs#L207)). Locked/active, malformed, or unrecognized session candidates are likewise skipped without an error notice. `Profile root was not created` is the defensive `None` guard after `OwnedRoot::open(..., true)`, which currently cannot return `None`. `Profile root has no parent` cannot occur for the production root formed by joining a fixed filename to the temporary directory. `Temporary profile child escaped its parent` is a defensive consistency check against a path returned by `read_dir`; the current standard-library path construction provides that parent. These guards are source text, not distinct demonstrated user-reachable screens.

### Bookmarks

Loading `bookmarks.json` happens during session setup (S). If absent, default bookmarks are created and persisted, so write errors can also be S. Adding a bookmark or removing an existing bookmark persists the store and can produce N. Removing a nonexistent ID does not write and produces no storage error. UI command validation and page JavaScript validation are covered by the browser/UI catalogue.

| Exact error/template | Trigger | Source |
| --- | --- | --- |
| `Failed to resolve local application data directory` | Cannot resolve application configuration location. S. | [store.rs:67](../../src/bookmarks/store.rs#L67) |
| `Failed to create config directory: <io-error>` | Initial config-directory creation fails. S. | [store.rs:71](../../src/bookmarks/store.rs#L71) |
| `Failed to create parent directory for bookmarks: <io-error>` | Store's parent-directory creation fails. S. | [store.rs:88](../../src/bookmarks/store.rs#L88) |
| `Failed to read bookmarks file: <io-error>` | Existing bookmarks cannot be read as UTF-8 text. S. | [store.rs:109](../../src/bookmarks/store.rs#L109) |
| `Failed to parse bookmarks JSON: <json-error>` | Malformed JSON or invalid serialized bookmark field types/structure. S; JSON detail may include line/column. | [store.rs:112](../../src/bookmarks/store.rs#L112) |
| `Bookmarks contain an empty or duplicate identifier` | Loaded bookmark identifier is empty or repeats an earlier ID. S. | [store.rs:117](../../src/bookmarks/store.rs#L117) |
| `A bookmark title is required` | Loaded or newly added title is empty after trimming. S or N; loaded titles have no additional prefix. | [store.rs:240](../../src/bookmarks/store.rs#L240) |
| `Failed to create temporary bookmark file: <io-error>` | Cannot create unique staging file. S or N. | [store.rs:149](../../src/bookmarks/store.rs#L149) |
| `Failed to write bookmark payload: <io-error>` | Staged JSON write fails. S or N. | [store.rs:151](../../src/bookmarks/store.rs#L151) |
| `Failed to sync bookmark file to storage: <io-error>` | Staged file sync fails. S or N. | [store.rs:153](../../src/bookmarks/store.rs#L153) |
| `Failed to atomically replace bookmark file: <io-error>` | Rename of staging file to destination fails. S or N. | [store.rs:156](../../src/bookmarks/store.rs#L156) |

Saved URL validation at [store.rs:120](../../src/bookmarks/store.rs#L120) prefixes the shared navigation validator error with exactly `Invalid saved bookmark: `. The complete S messages are:

| Complete exact saved-bookmark error | Trigger / validator source |
| --- | --- |
| `Invalid saved bookmark: Web addresses and searches must not contain control characters` | Control character remains after trimming. [navigation.rs:72](../../src/browser/navigation.rs#L72) |
| `Invalid saved bookmark: Web addresses must not contain backslashes` | URL contains a backslash. [navigation.rs:16](../../src/browser/navigation.rs#L16) |
| `Invalid saved bookmark: Invalid web address: <url-parse-error>` | URL parsing fails. [navigation.rs:18](../../src/browser/navigation.rs#L18) |
| `Invalid saved bookmark: Only HTTP and HTTPS web addresses are supported` | Unsupported scheme or no host. [navigation.rs:20](../../src/browser/navigation.rs#L20) |
| `Invalid saved bookmark: Web addresses containing a username or password are not supported` | Parsed username or password present. [navigation.rs:24](../../src/browser/navigation.rs#L24) |

Adding a bookmark calls the same validator without the `Invalid saved bookmark: ` prefix. Those raw messages can therefore be N (subject to earlier UI normalization/validation). The table above explicitly distinguishes saved-file messages from those direct navigation/addition messages.

`Failed to serialize bookmarks: <json-error>` exists at [store.rs:135](../../src/bookmarks/store.rs#L135). With the current fixed serializable structs/strings/enums and in-memory `to_string_pretty`, no ordinary error-returning input is apparent; allocation failure would not take this branch. Treat it as a defensive mapped error, not an independently reachable corrupt-file screen. Failure to remove a staging file after an error is ignored and adds no message.

### Website permission storage

Opening and reading `permissions.json` happens during session setup (S). A missing file is written with defaults, so staging/write errors can be S. In-session changes to popup default, download default, printing permission, a specific site rule, or removal of an existing site rule write the file (N). Choosing a remembered grant in the permission prompt writes a site rule and can also show its failure inline in that prompt. Unchanged setters and removal of a nonexistent rule do not write. The whole-store `reset()` method has no current application call site and is not a separate user-facing trigger.

| Exact error/template | Trigger | Source |
| --- | --- | --- |
| `Cannot resolve the application configuration directory` | Config location unavailable. S. | [permissions.rs:108](../../src/browser/permissions.rs#L108) |
| `Cannot create permission directory: <io-error>` | Parent directory cannot be created. S. | [permissions.rs:119](../../src/browser/permissions.rs#L119) |
| `Cannot open website permissions: <io-error>` | Existing file cannot be opened; missing-file case uses defaults instead. S. | [permissions.rs:124](../../src/browser/permissions.rs#L124) |
| `Cannot read website permissions: <io-error>` | File read fails. S. | [permissions.rs:360](../../src/browser/permissions.rs#L360) |
| `Saved website permissions exceed the storage limit` | Loaded file exceeds 262,144 bytes. S. | [permissions.rs:362](../../src/browser/permissions.rs#L362) |
| `Invalid saved website permissions: <json-error>` | Invalid JSON, field types, unknown/required fields, or enum values rejected by deserialization. S. | [permissions.rs:365](../../src/browser/permissions.rs#L365) |
| `Unsupported website permissions schema version: <version>` | Parsed version is not 1; `<version>` is displayed numerically. S. | [permissions.rs:368](../../src/browser/permissions.rs#L368) |
| `Saved website permissions contain too many rules` | Loaded list has more than 1,024 rules. S. | [permissions.rs:373](../../src/browser/permissions.rs#L373) |
| `Saved website permission rules must contain canonical origins without paths or credentials` | Saved origin normalizes successfully but differs from its canonical origin string, such as a path, trailing slash, or noncanonical spelling. S. Credentials generally fail earlier with the leaf below. | [permissions.rs:384](../../src/browser/permissions.rs#L384) |
| `Saved website permissions contain duplicate origin/capability rules` | The same canonical origin and permission capability occurs twice. S. | [permissions.rs:394](../../src/browser/permissions.rs#L394) |
| `Website permission limit reached (1024 rules)` | A proposed change would persist more than 1,024 rules. N. | [permissions.rs:270](../../src/browser/permissions.rs#L270) |
| `Website permissions exceed the storage limit` | Pretty-printed proposed JSON exceeds 262,144 bytes. N. | [permissions.rs:276](../../src/browser/permissions.rs#L276) |
| `Cannot stage website permissions: <io-error>` | Cannot create unique staging file. S or N. | [permissions.rs:286](../../src/browser/permissions.rs#L286) |
| `Cannot write website permissions: <io-error>` | Staging-file write fails. S or N. | [permissions.rs:288](../../src/browser/permissions.rs#L288) |
| `Cannot flush website permissions: <io-error>` | Staging-file sync fails. S or N. | [permissions.rs:290](../../src/browser/permissions.rs#L290) |
| `Cannot replace website permissions: <io-error>` | Staging-file rename to destination fails. S or N. | [permissions.rs:293](../../src/browser/permissions.rs#L293) |

`Cannot serialize website permissions: <json-error>` exists at [permissions.rs:274](../../src/browser/permissions.rs#L274). Like bookmark serialization, it is a defensive mapped error with no ordinary failure input in the current serializable snapshot shape. Failure to remove a staging file after an error is ignored and adds no message.

#### Permission-origin validation

These errors propagate **unchanged** from `normalize_origin` when loading a saved rule (S); there is no `Invalid saved website permissions: ` prefix for them. They can also be N when a site-rule command or remembered permission action passes an invalid address. Browser request paths that intentionally convert a validation failure to a block/drop do not show the error; those request-specific routes belong to the browser/UI catalogue.

| Exact error/template | Trigger | Source |
| --- | --- | --- |
| `Invalid website permission address` | Input exceeds 65,536 bytes, contains any control character, or contains a backslash. Check is before trimming. | [permissions.rs:306](../../src/browser/permissions.rs#L306) |
| `Invalid website permission address: <url-parse-error>` | Trimmed input fails URL parsing. | [permissions.rs:309](../../src/browser/permissions.rs#L309) |
| `Website permissions require an HTTP or HTTPS origin` | Parsed URL has another scheme or no host. | [permissions.rs:311](../../src/browser/permissions.rs#L311) |
| `Website permission addresses must contain an explicit HTTP(S) authority` | Input lacks literal `://`, even if the URL parser accepted it. | [permissions.rs:316](../../src/browser/permissions.rs#L316) |
| `Website permission addresses must not contain credentials` | Parsed username/password exists or the raw authority contains `@`, including an empty username. | [permissions.rs:322](../../src/browser/permissions.rs#L322) |
| `Website permissions cannot use wildcard hosts` | Parsed host contains `*`. | [permissions.rs:325](../../src/browser/permissions.rs#L325) |
| `Website permission origin is too long` | Canonical serialized origin exceeds 2,048 bytes. | [permissions.rs:329](../../src/browser/permissions.rs#L329) |

No storage message declares that a saved policy change succeeded after a failed persistence operation: permission state is committed in memory only after persistence succeeds. This catalogue records the strings and routing; it does not assert that arbitrary OS/library error text or browser-rendered notices can be fully enumerated as static English text.


## Windows, desktop, security and keyboard error-message catalogue

Read-only source audit, 2026-09-04. No failure cases were executed and no production source was changed for this catalogue. Braced placeholders identify the exact dynamic formatting slot; `{error}` is the underlying Windows/Tao/Wry display text, which can vary by operating-system language/version and failure code. This is a catalogue of application-authored text, not a claim that every defensive error has an ordinary user-triggerable reproduction.

### Where the messages actually appear

- **S — launcher/supervisor terminal error:** returned to `main`, always written as `[SafeBrowse] {error}` to stderr. A native **SafeBrowse error** dialog is additionally shown only when `StartupErrorPresentation::detect` selects `ConsoleAndDialog` (normally a standalone console launch). Existing terminals, redirected stderr, help and private worker invocations are console-only. See [src/startup_error.rs:25](../../src/startup_error.rs#L25); the startup/runtime section covers that presentation policy and truncation.
- **W — isolated worker terminal error:** only worker stderr, never its own native startup dialog. The supervisor does not transport this detailed text to a dialog. During initial authorization the supervisor may instead report its own handshake failure; after authorization it normally reports the generic nonzero worker-exit message from [src/main.rs:201](../../src/main.rs#L201). Therefore a failed clipboard, capture or native-window check inside a normal isolated worker must not be advertised as that exact detailed message appearing on its screen.
- **K — browser construction/teardown:** **S when `--windowed`**, **W in an isolated worker**. Construction errors occur before the browser becomes usable; they are not browser status notices.
- **I — running browser notice:** a returned event/command error is logged as `[SafeBrowse] {error}` and sent to `BrowserSession::notice` in both windowed and isolated modes ([src/ui/kiosk.rs:1611](../../src/ui/kiosk.rs#L1611)). The chrome status line receives the text and clears it after eight seconds ([src/ui/web/chrome.html:186](../../src/ui/web/chrome.html#L186)); internal Settings/Bookmarks can also receive their status updates. Rendering is best effort and may fail when the control itself is broken. This is not a modal error screen.
- **L — input-language picker:** `SELECT_INPUT_LANGUAGE` failures are displayed in the picker's status area with `window.showLanguageError` ([src/ui/kiosk.rs:1328](../../src/ui/kiosk.rs#L1328), [src/ui/web/language-picker.html:46](../../src/ui/web/language-picker.html#L46)). The handler first refreshes input-language state. If that refresh itself fails, its error becomes **I** and the original selection error is not displayed.
- **C — console only/nonfatal diagnostic:** explicitly logged and not passed to an error dialog or in-app notice.

### Screen-reachable native browser, shell and capture failures

| Exact message/template | Trigger | Route | Source |
| --- | --- | --- | --- |
| `Cannot load the SafeBrowse window icon: {error}` | Embedded native icon resource cannot be loaded by `Icon::from_resource`. | K | [src/ui/kiosk.rs:531](../../src/ui/kiosk.rs#L531), origin [src/ui/branding.rs:39](../../src/ui/branding.rs#L39) |
| `Cannot create window: {error}` | Main browser native window creation fails. | K | [src/ui/kiosk.rs:542](../../src/ui/kiosk.rs#L542) |
| `Cannot create SafeBrowse taskbar: {error}` | Isolated session's owned taskbar window cannot be created. This window is not created in windowed mode. | W | [src/ui/shell_windows.rs:66](../../src/ui/shell_windows.rs#L66), title supplied at [src/ui/kiosk.rs:572](../../src/ui/kiosk.rs#L572) |
| `Cannot create SafeBrowse input language: {error}` | Owned language-picker native window cannot be created. | K | [src/ui/shell_windows.rs:66](../../src/ui/shell_windows.rs#L66), title supplied at [src/ui/kiosk.rs:582](../../src/ui/kiosk.rs#L582) |
| `Cannot create floating keyboard: {error}` | Initially hidden floating-keyboard native window creation fails. It is created at startup even before detachment is requested. | K | [src/ui/floating_keyboard.rs:44](../../src/ui/floating_keyboard.rs#L44) |
| `Invalid window handle passed to CaptureProtector` | Capture setup receives a null HWND; defensive invariant check. | K | [src/security/capture.rs:26](../../src/security/capture.rs#L26) |
| `Failed to enable capture exclusion: {error}` | `SetWindowDisplayAffinity` fails on browser, taskbar, language-picker, floating-keyboard or an app-owned protected request prompt. These native windows are constructed at session startup. | K; isolated-only taskbar is W | [src/security/capture.rs:30](../../src/security/capture.rs#L30) |
| `Could not verify capture exclusion: {error}` | `GetWindowDisplayAffinity` readback fails after setting exclusion. | Same as preceding row | [src/security/capture.rs:33](../../src/security/capture.rs#L33) |
| `Capture exclusion was not applied (display affinity: {affinity:#x})` | Readback differs from `WDA_EXCLUDEFROMCAPTURE`; value is hexadecimal with `0x` prefix. | Same as preceding row | [src/security/capture.rs:36](../../src/security/capture.rs#L36) |
| `Cannot install controls on an invalid window` | Invalid HWND passed while installing the browser window procedure. | K | [src/ui/native.rs:32](../../src/ui/native.rs#L32) |
| `Native window proxy lock is unavailable` | Proxy mutex is poisoned when native browser controls are installed. | K | [src/ui/native.rs:36](../../src/ui/native.rs#L36) |
| `Native browser controls are already installed` | Second installation finds an existing native browser event proxy. | K | [src/ui/native.rs:38](../../src/ui/native.rs#L38) |
| `Could not install native browser controls: Win32 error {error_number}` | `SetWindowLongPtrW` returns zero with a nonzero captured last-error code; decimal code. | K | [src/ui/native.rs:51](../../src/ui/native.rs#L51) |
| `Cannot move on-screen keyboard: {error}` | Attach/Detach cannot reparent the existing keyboard WebView. | I | [src/ui/floating_keyboard.rs:80](../../src/ui/floating_keyboard.rs#L80), command propagation [src/ui/kiosk.rs:1312](../../src/ui/kiosk.rs#L1312) |
| `Cannot move floating keyboard: {error}` | Native drag initiation fails for a visible detached keyboard. | I | [src/ui/floating_keyboard.rs:130](../../src/ui/floating_keyboard.rs#L130), propagation [src/ui/kiosk.rs:1320](../../src/ui/kiosk.rs#L1320) |
| `{error}` with no SafeBrowse-specific prefix | Wry script evaluation fails while synchronizing the keyboard's Attach/Detach state. | I | [src/ui/floating_keyboard.rs:91](../../src/ui/floating_keyboard.rs#L91), callers [src/ui/kiosk.rs:1101](../../src/ui/kiosk.rs#L1101), [src/ui/floating_keyboard.rs:85](../../src/ui/floating_keyboard.rs#L85) |
| `Cannot resize browser controls: {error}` | A control bounds/visibility update fails; includes floating-keyboard layout's `set_bounds`/`set_visible`. | K on initial layout, I after resize/move/detachment/visibility changes | [src/ui/kiosk.rs:797](../../src/ui/kiosk.rs#L797), floating operations [src/ui/floating_keyboard.rs:117](../../src/ui/floating_keyboard.rs#L117) |
| `Cannot update browser window position: {error}` | WebView2 rejects `NotifyParentWindowPositionChanged` after browser/floating window movement. | I | [src/ui/kiosk.rs:898](../../src/ui/kiosk.rs#L898) |

Capture calls are skipped only for the explicit capture-allowed launch. The user's successful recorder test is unrelated to this text catalogue. `branding.rs` defines no additional application-written error text; its underlying `BadIcon` is wrapped by the browser/companion rows.

### Input-language errors

| Exact message/template | Trigger | Route | Source |
| --- | --- | --- | --- |
| `Could not activate the selected input language: {error}` | `ActivateKeyboardLayout` rejects the chosen installed layout. | L | [src/keyboard/language.rs:113](../../src/keyboard/language.rs#L113) |
| `The input language changed, but a page did not respond. Refocus the page and try again.` | A descendant input-window language-change message times out or is not delivered. | L | [src/keyboard/language.rs:130](../../src/keyboard/language.rs#L130) |
| `A page did not accept the selected input language. Refocus the page and try again.` | A descendant thread's actual keyboard layout does not match the requested layout. | L | [src/keyboard/language.rs:133](../../src/keyboard/language.rs#L133) |
| `Input languages can only be changed for a SafeBrowse window.` | Supplied input parent window is invalid or does not belong to the SafeBrowse process. Used by both snapshot and selection. | I for snapshot; L for selection if follow-up refresh succeeds | [src/keyboard/language.rs:186](../../src/keyboard/language.rs#L186) |
| `Could not read installed input languages: {windows_error}` | Either size discovery or list retrieval via `GetKeyboardLayoutList` returns no entries. | I during snapshot/layout refresh; L for selection if follow-up refresh succeeds | [src/keyboard/language.rs:268](../../src/keyboard/language.rs#L268), [src/keyboard/language.rs:276](../../src/keyboard/language.rs#L276) |
| `The selected input language is no longer installed. Reopen the language picker.` | The opaque requested layout ID does not match a currently installed layout. Also possible if a layout disappears between state snapshot and virtual-key generation. | L for selection; I for layout generation | [src/keyboard/language.rs:294](../../src/keyboard/language.rs#L294) |

[src/keyboard/osk.rs](../../src/keyboard/osk.rs) contributes no production error-screen strings. Its JSON string serialization has an invariant `expect` rather than a recoverable error; ordinary input editing deliberately returns silently for an unsupported/no-longer-editable field. Legacy cycling helpers are not used by the current production UI and are excluded.

### Elevation refusal and token-verification failures

These are **S** for public launch and **W** for a private worker launch. [src/main.rs:90](../../src/main.rs#L90) checks token integrity before dispatch. A valid public `--help` returns before this check. The following is the exact multiline administrator refusal body from [src/security/integrity.rs:114](../../src/security/integrity.rs#L114):

```text
SafeBrowse cannot start with administrator or high-integrity privileges.

Open SafeBrowse without "Run as administrator". If Windows also opens ordinary apps with administrator privileges, use a standard Windows account or enable User Account Control (UAC), restart Windows, and try again.

See README.md under Startup troubleshooting for the UAC repair command and restart steps.

No browsing session was started.
```

For an inability to verify the token, the exact outer message is `SafeBrowse could not verify that it is running without administrator privileges: {detail}. SafeBrowse will not start.` ([src/security/integrity.rs:103](../../src/security/integrity.rs#L103)). `{detail}` is exactly one of:

| Exact `{detail}` | Trigger | Source |
| --- | --- | --- |
| `Could not open the current process token: {error}` | `OpenProcessToken` fails. | [src/security/integrity.rs:89](../../src/security/integrity.rs#L89) |
| `Could not read token elevation: {error}` | Elevation `GetTokenInformation` fails. | [src/security/integrity.rs:156](../../src/security/integrity.rs#L156) |
| `Windows returned incomplete token elevation data` | Returned elevation structure is shorter than required. | [src/security/integrity.rs:159](../../src/security/integrity.rs#L159) |
| `Could not determine token integrity buffer size: {error_or_Windows returned no data}` | Integrity sizing result is too small; uses the actual probe error or literal `Windows returned no data`. | [src/security/integrity.rs:169](../../src/security/integrity.rs#L169) |
| `Could not read token integrity level: {error}` | Integrity-data `GetTokenInformation` fails. | [src/security/integrity.rs:192](../../src/security/integrity.rs#L192) |
| `Windows returned invalid token integrity data length` | Returned structure length is too short or exceeds allocated storage. | [src/security/integrity.rs:197](../../src/security/integrity.rs#L197) |
| `Windows returned an out-of-range integrity SID` | SID pointer/header lies outside returned buffer. | [src/security/integrity.rs:206](../../src/security/integrity.rs#L206) |
| `Token integrity SID length overflowed` | SID length arithmetic overflows. | [src/security/integrity.rs:214](../../src/security/integrity.rs#L214), [src/security/integrity.rs:216](../../src/security/integrity.rs#L216) |
| `Windows returned an invalid integrity SID length` | SID has zero subauthorities or extends beyond buffer. | [src/security/integrity.rs:218](../../src/security/integrity.rs#L218) |
| `Windows returned an invalid integrity SID` | `IsValidSid` rejects it. | [src/security/integrity.rs:221](../../src/security/integrity.rs#L221) |
| `Windows returned an out-of-range integrity RID` | RID pointer is null or outside the validated SID/buffer. | [src/security/integrity.rs:229](../../src/security/integrity.rs#L229) |

### Desktop setup, switching and supervision

| Exact message/template | Trigger | Route | Source |
| --- | --- | --- | --- |
| `Failed to open default desktop: Win32 Error {last_error:?}` | Both normal and reduced-access opens of `Default` fail; debug-formatted `WIN32_ERROR`. | S in supervisor, W when the authenticated worker opens it | [src/desktop/manager.rs:185](../../src/desktop/manager.rs#L185) |
| `The isolated session desktop was already initialized` | Duplicate create request on an initialized manager; defensive invariant. | S | [src/desktop/manager.rs:202](../../src/desktop/manager.rs#L202) |
| `Refusing to reuse a pre-existing isolated desktop` | Fresh UUID desktop name unexpectedly resolves to an existing desktop. | S | [src/desktop/manager.rs:219](../../src/desktop/manager.rs#L219) |
| `Could not verify a fresh isolated desktop: {error}` | Existence probe fails with an error other than file-not-found. | S | [src/desktop/manager.rs:225](../../src/desktop/manager.rs#L225) |
| `Failed to create the isolated session desktop: {error}` | `CreateDesktopW` fails. | S | [src/desktop/manager.rs:246](../../src/desktop/manager.rs#L246) |
| `Invalid desktop handle returned` | Creation/open reports success but returns an invalid handle. | S for create, W for worker open | [src/desktop/manager.rs:248](../../src/desktop/manager.rs#L248), [src/desktop/manager.rs:277](../../src/desktop/manager.rs#L277) |
| `Failed to open the authorized session desktop: {error:?}` | Authenticated worker cannot open its authorized desktop; debug formatting. | W | [src/desktop/manager.rs:274](../../src/desktop/manager.rs#L274) |
| `Safe desktop handle not initialized` | Switching/input-desktop query lacks the safe handle; defensive invariant. | S on initial switch; C during supervisor taskbar/toggle actions | [src/desktop/manager.rs:129](../../src/desktop/manager.rs#L129), [src/desktop/manager.rs:293](../../src/desktop/manager.rs#L293) |
| `Default desktop handle not initialized` | Default switch/input-desktop query lacks its handle; defensive invariant. | S on supervisor restoration; W on worker teardown; I on browser desktop switch; C on supervisor toggle | [src/desktop/manager.rs:133](../../src/desktop/manager.rs#L133), [src/desktop/manager.rs:331](../../src/desktop/manager.rs#L331) |
| `The desktop shortcut is unavailable while another Windows desktop is active` | Neither session nor Default is currently the input desktop, e.g. another Windows desktop is active. | C only during companion toggle | [src/desktop/manager.rs:140](../../src/desktop/manager.rs#L140), logged by [src/desktop/dock.rs:298](../../src/desktop/dock.rs#L298) |
| `Could not determine the active desktop: {error}` | `GetUserObjectInformationW(UOI_IO)` fails during companion toggle. | C | [src/desktop/manager.rs:403](../../src/desktop/manager.rs#L403), logged by [src/desktop/dock.rs:298](../../src/desktop/dock.rs#L298) |
| `Failed to switch to safe desktop after retries: Win32 Error {last_error:?}` | All ten `SwitchDesktop` attempts fail. | S on initial activation; C on companion/taskbar re-entry | [src/desktop/manager.rs:317](../../src/desktop/manager.rs#L317) |
| `Failed to switch to default desktop after retries: Win32 Error {last_error:?}` | All ten default-desktop switch attempts fail. | S in supervisor/fatal dock recovery; W during worker teardown; I on browser switch action; C on companion shortcut | [src/desktop/manager.rs:355](../../src/desktop/manager.rs#L355) |
| `Create the isolated session desktop before launching its worker` | Worker spawn requested without a pinned safe desktop; defensive invariant. | S | [src/desktop/manager.rs:385](../../src/desktop/manager.rs#L385) |
| `Could not initialize desktop watchdog: {error}` | Duplicating the worker process handle for watchdog fails. | S | [src/desktop/recovery.rs:77](../../src/desktop/recovery.rs#L77) |
| `Could not start desktop watchdog: {error}` | Background watchdog thread creation fails. | S | [src/desktop/recovery.rs:113](../../src/desktop/recovery.rs#L113) |
| `Could not read browser exit status: {error}` | Supervisor `GetExitCodeProcess` fails after session termination. | S | [src/desktop/launch_auth.rs:176](../../src/desktop/launch_auth.rs#L176) |

The two `SwitchDesktop` calls in the recovery guard/watchdog deliberately ignore their result ([src/desktop/recovery.rs:43](../../src/desktop/recovery.rs#L43), [src/desktop/recovery.rs:103](../../src/desktop/recovery.rs#L103)): they do not generate a separate error string. `assign_current_thread_to_safe_desktop` and its `Failed to assign thread to safe desktop...` error have no production caller and are excluded. `SupervisedWorkerProcess::contains_process` and its lifetime-container/containment errors are fixture-only callers and are excluded.

### Companion window failures (normal isolated mode only)

All rows below are **S**, returned after the companion/supervisor unwinds; none is a companion in-page notice. Main's final desktop-restoration error can take precedence over a returned companion error.

| Exact message/template | Trigger | Source |
| --- | --- | --- |
| `Cannot load the SafeBrowse companion icon: {error}` | Embedded companion icon cannot be loaded. | [src/desktop/dock.rs:205](../../src/desktop/dock.rs#L205) |
| `Failed to create Dock companion window: {error}` | Companion native window creation fails. | [src/desktop/dock.rs:210](../../src/desktop/dock.rs#L210) |
| `Failed to initialize Dock webview: {error}` | Companion WebView creation fails. | [src/desktop/dock.rs:248](../../src/desktop/dock.rs#L248) |
| `Cannot load the session control: {error}` | Loading bundled companion HTML fails. | [src/desktop/dock.rs:255](../../src/desktop/dock.rs#L255) |
| `Could not request browser shutdown: {error}` | Posting `WM_QUIT` to worker UI thread fails after session termination request or a companion engine failure. | [src/desktop/dock.rs:87](../../src/desktop/dock.rs#L87) |
| `Lost access to the supervised browser process` | Polling the worker process returns `WAIT_FAILED`. | [src/desktop/dock.rs:318](../../src/desktop/dock.rs#L318) |
| `Browser shutdown timed out; the worker was stopped. Temporary data may remain.` | Graceful-shutdown deadline expires and termination is attempted. This text does not validate successful termination. | [src/desktop/dock.rs:325](../../src/desktop/dock.rs#L325) |
| `{failure_message} The session control failed, so SafeBrowse is closing the session. Check the website's transaction status before trying again.` | A monitored companion browser/renderer process fails or becomes unresponsive. | [src/desktop/dock.rs:268](../../src/desktop/dock.rs#L268) |

For the final row, `{failure_message}` is exactly one of the following, with its existing final period preserved ([src/browser/health.rs:33](../../src/browser/health.rs#L33)):

1. `The browser engine stopped unexpectedly.`
2. `A page stopped unexpectedly.`
3. `A page stopped responding.`
4. `Part of a page stopped unexpectedly.`
5. `The browser reported an unexpected process failure.`

The companion also propagates runtime-environment validation, engine-monitor attachment and profile errors without adding another wrapper; those are catalogued by their owning modules. Additional desktop-switch/shutdown errors can be appended to the engine-failure body with `\n\n`; a profile-cleanup failure is also appended with `\n\n` ([src/desktop/dock.rs:272](../../src/desktop/dock.rs#L272), [src/desktop/dock.rs:277](../../src/desktop/dock.rs#L277), [src/desktop/dock.rs:351](../../src/desktop/dock.rs#L351)). A later lost-process or timeout condition can replace an earlier companion error, so not every simultaneous condition remains in the final text.

### Worker creation and private authorization

**S/W** means the helper is called by both the supervisor and worker. An S copy can reach the public launcher's presentation policy; a W copy remains worker stderr. Private-argument parsing itself is console-only because the presence of `--worker` or `--worker-auth-*` disables a dialog. The handshake sends authorization records only; it does not forward worker diagnostic strings.

| Exact message/template | Trigger | Route | Source ([src/desktop/launch_auth.rs](../../src/desktop/launch_auth.rs)) |
| --- | --- | --- | --- |
| `Worker authorization arguments cannot be repeated` | Duplicate private transport option. | W/private C | 102 |
| `Worker authorization arguments are incomplete` | Missing argument value or missing one of the three handles. | W/private C | 106, 125 |
| `Worker authorization handles must be distinct` | Repeated handle address among transport fields. | W/private C | 117 |
| `Worker authorization handle address is invalid` | Non-numeric, zero or all-ones handle address. | W/private C | 762, 764 |
| `Invalid isolated session desktop identity` | Desktop namespace/UUID version/length validation fails. Supervisor supplies a generated value, so its branch is defensive. | S/W | 249, 255 |
| `Could not locate the browser executable: {error}` | Supervisor cannot obtain `current_exe`. | S | 268 |
| `Could not create the authorized browser worker: {error}` | `CreateProcessW` with startup attributes fails. | S | 324 |
| `The contained browser worker had an unexpected suspension state` | `ResumeThread` does not report exactly one previous suspension. | S | 345 |
| `Browser worker failed its single-use launch challenge` | Worker acknowledgment bytes do not match expected PID/nonce. | S | 355 |
| `Worker authorization received an invalid kernel handle` | Kernel-handle wrapper receives an invalid value. | S/W | 368 |
| `Worker authorization requires inherited kernel handles` | `GetHandleInformation` rejects a supplied handle. | W | 377 |
| `Could not restrict worker handle inheritance: {error}` | `SetHandleInformation` fails. | S/W | 391 |
| `Could not size the worker inheritance policy` | Attribute-list sizing returns zero bytes. | S | 417 |
| `Could not initialize worker inheritance policy: {error}` | Attribute-list initialization fails. | S | 424 |
| `Could not restrict the worker's inherited handles: {error}` | Setting the explicit inheritance handle list fails. | S | 440 |
| `Could not bind the worker to its supervisor lifetime: {error}` | Setting atomic job-list startup attribute fails. | S | 454 |
| `Could not create private worker authorization channels: {error}` | `CreatePipe` fails. | S | 482 |
| `Could not create supervisor identity capability: {error}` | Duplicating restricted supervisor identity handle fails. | S | 502 |
| `Could not create the worker lifetime container: {error}` | Creating unnamed job object fails. | S | 509 |
| `Could not enforce supervised worker lifetime: {error}` | Setting kill-on-job-close fails. | S | 521 |
| `Worker authorization has no valid supervisor identity` | Parent handle has no PID or refers to worker itself. | W | 532 |
| `Worker authorization requires private supervisor pipes` | Querying pipe creator PID fails. | W | 538 |
| `Worker authorization channels belong to another process` | Pipe creator does not match supervisor PID. | W | 540 |
| `Worker was not created by its authorization supervisor` | Recorded parent PID differs from the supplied supervisor. | W | 544 |
| `Could not verify worker executable identity: {error}` | Worker cannot resolve its own executable path. | W | 548 |
| `Worker supervisor does not use the same executable` | Worker and supervisor executable file identities differ. | W | 550 |
| `Could not verify worker parent process: {error}` | Process snapshot creation fails. | W | 560 |
| `Could not inspect worker parent process: {error}` | Reading first process snapshot entry fails. | W | 567 |
| `Could not locate the worker's parent process` | Process snapshot iteration ends/fails without finding worker. | W | 574 |
| `Could not verify supervisor executable identity: {error}` | Querying supervisor process image path fails. | W | 590 |
| `Could not open executable for identity verification: {error}` | Opening either executable file fails. | W | 604 |
| `Could not read executable file identity: {error}` | File identity query fails. | W | 607 |
| `Could not verify the worker desktop assignment: {error}` | `GetThreadDesktop` fails. | W | 617 |
| `Could not read the worker desktop identity: {error}` | Reading current desktop name fails. | W | 629 |
| `Worker desktop identity is unterminated` | Desktop name buffer has no NUL terminator. | W | 633 |
| `Worker desktop identity is invalid Unicode` | Desktop name cannot decode as UTF-16. | W | 635 |
| `Worker authorization peer exited before startup completed` | Peer process already signaled exit during handshake. | S/W | 641 |
| `Worker authorization peer could not be verified` | Peer wait returns neither running nor exited. | S/W | 642 |
| `Worker authorization timed out` | Five-second handshake deadline expires. | S/W | 658 |
| `Worker authorization channel closed before completion` | Pipe availability probe fails. | S/W | 662 |
| `Worker authorization channel could not be read` | `ReadFile` fails. | S/W | 677 |
| `Worker authorization channel ended before completion` | A read returns zero bytes before the required record completes. | S/W | 679 |
| `Worker authorization record exceeds its channel capacity` | Outgoing record exceeds 4096 bytes; defensive with current bounded records. | S/W | 688 |
| `Worker authorization channel could not be written` | `WriteFile` fails. | S/W | 692 |
| `Worker authorization channel accepted an incomplete record` | Pipe accepts fewer bytes than required. | S/W | 694 |
| `Unsupported worker authorization protocol` | Hello magic/version mismatch. | W | 711 |
| `Worker authorization desktop identity has an invalid length` | Hello name length is zero or over 96 bytes. | W | 715 |
| `Worker authorization was issued for a different process` | Hello supervisor/worker PID mismatch. | W | 216 |
| `Worker authorization contains an invalid desktop identity` | Received desktop name is not UTF-8. | W | 221 |
| `Worker was not created on its authorized desktop` | Actual worker desktop differs from authorized name. | W | 224 |
| `Supervisor did not confirm the worker authorization` | Commit record does not match expected magic/nonce. | W | 231 |
| `Worker command line exceeds the Windows length limit` | Serialized UTF-16 command exceeds 32,767 units. | S | 794 |
| `Worker arguments cannot contain null characters` | Argument quoting encounters a NUL; defensive because ordinary OS arguments cannot contain embedded NUL. | S | 808 |

### Clipboard errors: worker stderr, not normal isolated error dialogs

`ClipboardBroker` is called by production only from `run_worker`, before the browser session and after it ([src/main.rs:118](../../src/main.rs#L118), [src/main.rs:126](../../src/main.rs#L126)). Windowed launches do not clear the clipboard. Each row is therefore **W**, not K or I:

| Exact message/template | Trigger | Source |
| --- | --- | --- |
| `Could not open the Windows clipboard: {error}` | `OpenClipboard` still fails after five attempts. | [src/security/clipboard.rs:27](../../src/security/clipboard.rs#L27) |
| `Could not empty the Windows clipboard: {error}` | `EmptyClipboard` fails after open; close is still attempted. | [src/security/clipboard.rs:36](../../src/security/clipboard.rs#L36) |
| `Could not release the Windows clipboard: {error}` | `CloseClipboard` fails after a successful empty. | [src/security/clipboard.rs:37](../../src/security/clipboard.rs#L37) |

If both the browsing session and final clipboard cleanup fail, worker stderr combines them as `{session_error}\n\nClipboard cleanup also failed: {clipboard_error}` ([src/main.rs:128](../../src/main.rs#L128)). The startup/runtime section owns the supervisor's separate generic exit message. If both empty and close fail, the empty error wins because of `?` ordering.

### Nonfatal console-only diagnostics: no application error screen

| Exact emitted log/template | Trigger | Source |
| --- | --- | --- |
| `[SafeBrowse] PrintScreen shortcut unavailable: {detail}` | PrintScreen hotkey registration fails; session continues with capture affinity. | [src/ui/kiosk.rs:563](../../src/ui/kiosk.rs#L563) |
| `[SafeBrowse] Desktop shortcut unavailable: {detail}. Use the taskbar entry instead.` | Companion Ctrl+Alt+D hotkey registration fails; session continues. | [src/desktop/dock.rs:220](../../src/desktop/dock.rs#L220) |
| `[SafeBrowse] {desktop_switch_or_query_error}` | Companion taskbar re-entry/toggle failure; see desktop table above. | [src/desktop/dock.rs:290](../../src/desktop/dock.rs#L290), [src/desktop/dock.rs:298](../../src/desktop/dock.rs#L298) |
| `[SafeBrowse] Floating language-bar suppression unavailable: {detail}` | Installing the optional isolated-desktop indicator guard fails; browser continues. | [src/ui/kiosk.rs:522](../../src/ui/kiosk.rs#L522) |
| `[SafeBrowse] Cannot refresh floating language-bar suppression: {detail}` | Refreshing the optional indicator guard fails; normal language snapshot is still attempted. | [src/ui/kiosk.rs:972](../../src/ui/kiosk.rs#L972) |
| `[SafeBrowse] Could not remove the input-indicator visibility hook` | Unhooking during guard destruction fails. | [src/keyboard/language_bar.rs:196](../../src/keyboard/language_bar.rs#L196) |
| `[SafeBrowse] Native TSF caption lookup is unavailable; floating indicators may remain visible` | Optional `InternalGetWindowText` export is unavailable. | [src/keyboard/language_bar.rs:291](../../src/keyboard/language_bar.rs#L291) |

The two hotkey `{detail}` families are exactly `Invalid window handle` ([src/security/hooks.rs:45](../../src/security/hooks.rs#L45), `:67`), `{shortcut} is already assigned to another application` (`:104`), or `Could not register {shortcut}: {error}` (`:106`). `{shortcut}` is precisely `PrintScreen` or `Ctrl+Alt+D`. The latter companion wrapper always adds its final sentence, including a period immediately after the dynamic detail.

Indicator suppression `{detail}` is exactly one of:

| Exact detail | Trigger | Source |
| --- | --- | --- |
| `Input indicator suppression is already installed on this thread` | Existing thread-local indicator guard. | [src/keyboard/language_bar.rs:147](../../src/keyboard/language_bar.rs#L147) |
| `Cannot monitor the isolated desktop's input indicators` | `SetWinEventHook` returns invalid hook. | [src/keyboard/language_bar.rs:167](../../src/keyboard/language_bar.rs#L167) |
| `Cannot inspect SafeBrowse's input indicators: {error}` | `EnumDesktopWindows` fails during installation/refresh. | [src/keyboard/language_bar.rs:188](../../src/keyboard/language_bar.rs#L188) |
| `Cannot read the input thread's desktop: {error}` | Current input-thread desktop lookup fails during guard installation/refresh. | [src/keyboard/language_bar.rs:205](../../src/keyboard/language_bar.rs#L205) |
| `Cannot identify the input thread's desktop: {error}` | Desktop-name lookup fails. | [src/keyboard/language_bar.rs:216](../../src/keyboard/language_bar.rs#L216) |
| `Input indicator suppression requires the isolated SafeBrowse desktop` | Actual desktop name differs from the expected authenticated session name. | [src/keyboard/language_bar.rs:225](../../src/keyboard/language_bar.rs#L225) |

Per-window identity probes reuse the last three checks but swallow their errors while excluding the unmatched window; that path produces no log or screen. The informational `Hiding duplicate input indicator...` message is not an error and is excluded. Other intentionally ignored cleanup/window-position APIs do not create hidden extra user-facing strings.
