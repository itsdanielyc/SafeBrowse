# Security review follow-up — 4 September 2026

This follow-up implements bounded corrections to findings 1–5 in the [source review](security-review-2026-09-04.md). It does not establish Bitdefender Safepay equivalence, resistance to malware already running as the same Windows user, or the integrity of existing release executables. The earlier review and its SHA-256 manifest describe the **pre-remediation** snapshot and remain historical evidence.

## 1. Production website printing integration

The actual `build_content_view` path now installs the document-created print wrapper with subframe injection. This covers normal website tabs and the same-environment builder used for approved popups. The misplaced wrapper was removed from trusted controls. The existing host Printing setting remains default-off and gates toolbar/shortcut printing.

A native regression uses the production builder, hidden windows, a disposable profile and loopback HTML. It inspects the wrapper in the top document, an iframe, and a popup-style view, and invokes printing only after identifying the wrapper. This catches the original wiring defect that standalone JavaScript tests could not detect.

**Remaining limitation:** this is best-effort suppression. The previously observed opener-retained native `print` reference is not fixed by this change. A builder test with an inherited environment is not evidence that every `window.open()` timing case is blocked. Built-in PDF and printer/save dialog behavior remain separate test surfaces. This is not an engine-enforced global printing prohibition.

## 2. Native engine failure handling

`BrowserHealthMonitor` subscribes to `ProcessFailed`, `BrowserProcessExited`, and `NewBrowserVersionAvailable`. Each production website, popup, trusted control and companion retains its monitor until before view destruction. Partial registration failures unsubscribe. Callbacks queue application events and never reload pages or execute a payment retry.

Browser exits, renderer exits/unresponsiveness, frame renderer exits, missing failure information and unknown process failures cause a safe session exit. Pending permission requests and downloads are cancelled. GPU and utility failures documented as automatically recovered are left to WebView2. Installed-update events produce a persistent **Restart to update** notice in the title bar, without interrupting an in-progress transaction.

For worker failures, the supervisor observes worker exit and attempts to restore Windows. For companion failures, it attempts restoration immediately and requests orderly worker shutdown before the bounded forced-termination fallback. The orderly deadline includes the ten-second profile cleanup budget plus five seconds for teardown. Secondary clipboard/profile cleanup errors are preserved. Bundled HTML is loaded only after its environment has been validated and monitoring attached.

The native crash fixture opens a handle to the browser PID belonging to its own disposable profile, confirms the unique fixture is still running after obtaining that handle, and terminates only that process. It checks that the production health event arrives and that the loopback page was not requested again. It does not terminate a real SafeBrowse session, switch desktops, or use banking data.

**Remaining limitation:** this improves response to engine events when the host event loop can run. It does not detect every application deadlock, guarantee recovery after supervisor termination, independently restore Windows after power loss, or guarantee cancellation of an already submitted transaction. Full isolated-desktop fault testing and actual renderer-hang injection remain outstanding. The unused compatibility `BrowserController` returns a bare WebView; engine monitoring belongs to the production session wrappers.

## 3. Same-user malware boundary

No new isolation claim has been added. The existing host still runs as the current Windows user. Desktop access controls and capture affinity do not make it a separate security principal. A same-user SID-only ACL would not exclude malware using the same SID.

The larger solution requires a reviewed isolation architecture, such as a separate security principal with narrowly brokered capabilities or VM-backed browsing, followed by compatibility and adversarial validation. Clipboard, downloads, input, printing, updates and recovery would all need explicit boundaries. Selecting and implementing that architecture is separate work; silently changing token/desktop permissions here would not prove it safe or usable.

## 4. Runtime selection and temporary storage

Before launcher session effects, runtime preflight rejects documented environment overrides for executable/profile folders, additional arguments, channel selection and debugging, plus applicable current and legacy loader policies. Both Windows registry views and HKCU/HKLM are inspected for the executable, application identities and supported wildcard selectors. Policy read failures are reported; values are not echoed and settings are never rewritten.

The loader-selected runtime and each created environment must report a numeric stable version at least `151.0.4129.107`. This is the project's exercised compatibility floor, not a claim that the version has all current security fixes. Each created environment must resolve to the intended user-data directory before document navigation. These checks do not authenticate Microsoft's installed files or prevent concurrent same-user host tampering.

New temporary profiles live under `%TEMP%\SafeBrowse_EphemeralProfiles_v1`, with root/session ownership records and exclusive active-session locks. Normal cleanup releases browser objects and retries deletion. The next launcher, after acquiring its session mutex, conservatively scans this owned root for abandoned profiles. Directory ancestors are pinned; deletion targets opened file handles, and reparse points and multiply linked files are rejected. Legacy, unmarked, malformed, active and persistent paths are left alone.

Reclamation is bounded to three seconds, 128 root entries, 50,000 child objects and depth 64. Individual slow OS calls can exceed the time budget. Errors or scan exhaustion block a new launcher session and report the reason; skipped entries are not evidence of successful deletion. Explicit cleanup retries remain possible, while destructor cleanup does not repeat a previously exhausted delay. This is deletion, not forensic secure erasure.

## 5. Source, dependency and release provenance

Added a pinned Rust toolchain, Windows GitHub verification workflow, weekly dependency/action update proposals, a Windows dependency advisory checker, and release-candidate tooling. Actions are pinned to full official commit IDs. PR verification has read-only repository permissions.

The candidate script requires a complete clean committed tree, exports that revision with Git, builds in a fresh target directory, and packages checksums plus source/compiler/lockfile/build metadata. A manually dispatched workflow separates verification and build from an attestation-only job that does not execute project code. Tooling fixtures use a disposable repository and mocked build/audit outputs; their fake executable is not a release.

**Remaining limitation:** no GitHub workflow was run, public release published, Authenticode signature created, or existing binary retroactively verified. The surrounding checkout remains uncommitted, so a real candidate is intentionally rejected until its owner reviews and commits the complete source. Build tooling, the host, Cargo configuration/cache, SDK and upstream native code remain trusted inputs. Byte-for-byte reproducibility is not established. See [RELEASING.md](../RELEASING.md).

## Verification record

Execution results are recorded in [VALIDATION.md](../VALIDATION.md). Verification uses disposable content and hidden native fixtures. No normal banking session, physical print job, or protected capture workflow was launched for these corrections.

The [follow-up SHA-256 manifest](security-remediation-2026-09-04.sha256) identifies the application, build, test, asset, release-tooling and selected documentation files after remediation. A separate local review build is at `target/security-remediation/release/safebrowse.exe`; its digest and build command are recorded in `VALIDATION.md`. It remains unsigned. This manifest is an integrity snapshot, not publisher authentication or an attestation.

Important remaining release checks:

1. On disposable Windows systems, verify isolated desktop restoration after website, renderer, companion, worker and supervisor failures, including slow storage cleanup and failed desktop switches.
2. Exercise capture exclusion and the recording override independently, including native printer/save dialogs, frames, popup timing and unsupported printing paths.
3. Run the committed-revision CI/candidate workflow on the real repository, verify artifact attestations, review signing/distribution and support ownership, and obtain independent architecture/security review before making production security claims.

Platform references: [WebView2 process-related events](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-related-events), [WebView2 environment creation](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2-idl#createcorewebview2environmentwithoptions), [WebView2 policies](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-webview-policies), and [Windows desktop security](https://learn.microsoft.com/en-us/windows/win32/winstation/desktop-security-and-access-rights).
