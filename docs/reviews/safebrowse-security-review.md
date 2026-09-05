# SafeBrowse source security review

**Date:** 2026-09-03  
**Reviewed state:** working tree based on `c288c11`, including the current native permission broker, UI changes, and subsequent startup/security remediations. Symbol references identify current code; numeric references retained from the original review can move as the working tree changes.  
**Method:** bounded source review of the Rust application, bundled JavaScript/HTML, Wry 0.56.1's Windows implementation, and relevant Microsoft platform documentation. Follow-up review checks the startup guards and native browser settings. Current regression commands, results, and unverified native scenarios are recorded in [VALIDATION.md](../VALIDATION.md). No real banking accounts or hostile software were used, and no comprehensive malware-resistance assessment was performed.

## Assessment

The application has useful controls around remote content: separate trusted UI views, constrained navigation, an exact-origin permission store, deferred native permission decisions, capture exclusion checks, and a clear recording override. Follow-up changes remove the identified name-based session substitution path, require a private authenticated worker launch, reject elevated hosting, and explicitly require WebView2 reputation checking. These do not establish a malware-resistant banking environment. The ordinary host still runs as the launching Windows user, with no separate host security principal or containment boundary against same-user malware.

This review does not establish equivalence to Bitdefender Safepay, prove absence of vulnerabilities, or constitute an independent audit or certification. The reviewer also contributed implementation code in this working tree. Vendor claims and their limits are tracked separately in [the Safepay research note](safepay-research.md).

Severity below is relative to the proposed security-browser use case. Local findings require a process already running in the user's interactive session; they are not remote code-execution findings.

## Prioritized findings

| ID | Severity / confidence | Status | Finding |
| --- | --- | --- | --- |
| SB-01 | High / high | Remediated in source; residual local-access limits below | Named session markers and a fixed desktop could be substituted by another local process. |
| SB-02 | Medium / high | Remediated in source | Direct worker invocation bypassed supervision and single-session enforcement. |
| SB-03 | Medium, conditional / high | Remediated in source | An elevated launcher could create an elevated browser host. |
| SB-04 | Low, availability / high | Open | Recovery detects worker exit, not responsiveness, and does not survive supervisor termination. |
| SB-05 | Low, defense in depth / high | Remediated; native test passed | Wry kept website web messaging enabled even without an application IPC handler. |
| RG-01 | Release-readiness gap | Open | An authenticated release/update and dependency-maintenance process is not implemented in this repository. |

### SB-01: unauthenticated desktop/session rendezvous

**Locations:** `src/main.rs::run_launcher` and `run_supervisor`; `src/desktop/manager.rs::new`, `create_safe_desktop`, and `from_authenticated_worker`; `src/desktop/launch_auth.rs`.

**Original issue:** the launcher treated `Local\SafeBrowse_Session_Mutex` plus another fixed marker as authority to open and switch to `SafeBrowseDesktop`. The windowed fallback similarly located a window by its title. Another local process could precreate those objects and influence where the legitimate launcher sent the user. This was a source-derived substitution path, not an executed credential-theft exploit.

**Remediation applied:** every new supervisor generates a UUIDv4 desktop name, refuses a discovered preexisting object, fails closed when the preflight cannot establish absence, and retains the created desktop handle. There is no fallback to a previous desktop. The worker receives that identity through the authenticated exchange described in SB-02 and confirms its actual assigned desktop. The launcher no longer re-enters a session through fixed markers or window titles: any second launch returns an error, and the already-running companion uses its existing session handles.

**Residual limits:** `CreateDesktopW` has no documented atomic exclusive-create operation. The preflight and creation are separate calls; the fresh unpredictable name and retained handle remove predictable-name reuse, but are not a defense against an attacker able to inspect or modify the supervisor. Desktop access controls still inherit from the window station. Requested desktop rights describe what SafeBrowse asks to open, not what another process is forbidden to obtain. A separate desktop is not a separate security principal. [CreateDesktopW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createdesktopw), [desktop access rights](https://learn.microsoft.com/en-us/windows/win32/winstation/desktop-security-and-access-rights).

The predictable single-session mutex remains an availability limit: another local process can precreate it and cause startup to refuse. It no longer authenticates a session or selects a desktop/window. Microsoft documents precreation attacks against fixed single-instance mutexes. [CreateMutexW](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw).

**Test surface:** fresh desktop identities must differ; preexisting session desktops must be rejected; duplicate launches must fail without selecting another process's desktop or window. Same-user access and adversarial creation-race tests require a disposable Windows environment and a clearly defined local-adversary model. See [VALIDATION.md](../VALIDATION.md) for executed cases and remaining checks.

### SB-02: public worker mode bypasses lifecycle enforcement

**Locations:** `src/main.rs::parse_launch_request` and `run_worker`; `src/desktop/launch_auth.rs::authenticate_worker_launch`, `validate_supervisor`, and `spawn_authenticated_worker`; `src/desktop/manager.rs::from_authenticated_worker`.

**Original issue:** `--worker` was accepted without a supervisor handshake or session capability. The worker path opened the shared desktop, cleared the clipboard, and started the kiosk before acquiring a session lock. A local caller could create unsupervised or additional workers, breaking lifecycle and single-writer assumptions without acquiring a new Windows privilege.

**Remediation applied:** worker mode requires one complete internal transport; public launches cannot supply it. The supervisor inherits only two anonymous-pipe endpoints and a limited query/synchronization process handle into its child through an explicit handle list. The fresh challenge and desktop identity travel in the pipe protocol, not command-line arguments or logs. The child verifies a live supervisor, both pipe owners, its recorded OS parent, the parent's executable file identity, its exact worker PID, and its assigned desktop. It acknowledges the challenge and waits for the matching supervisor commit before application desktop, clipboard, profile, or UI work. The exchange is bounded, and its handles are made non-inheritable in the worker.

The worker is placed in an unnamed kill-on-close job before it executes. The job handle stays with the supervisor, so ordinary supervisor termination also ends its job members. Missing, invalid, stale, wrong-parent, or incomplete exchanges fail before browsing starts. This enforces one launch relationship; it is not a general defense against injection into a trusted process, executable tampering, or theft/duplication of handles by another sufficiently privileged same-user process. Job termination can leave temporary profile data and does not independently restore the Windows desktop.

**Test surface:** standalone or malformed worker invocation, wrong-parent or stale transport, wrong PID/nonce, absent final commit, and supervisor exit during startup must fail without touching an existing session's application state. Lifetime tests must confirm job members cannot survive ordinary supervisor closure. The current native fixtures and their execution limits are listed in [VALIDATION.md](../VALIDATION.md).

### SB-03: elevation is inherited by the browser host

**Locations:** `src/main.rs::run`; `src/security/integrity.rs::refuse_elevated_browser_host`, `current_process_integrity`, and `classify_integrity_rid`.

**Original issue:** the isolated worker was created with ordinary `CreateProcessW`; windowed mode hosted WebView2 directly in the launcher. No guard rejected elevation, so an administrator launch gave the Rust host an administrator token. Windows starts an ordinary child process in the caller's security context. [CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw).

**Precondition and impact:** a user launched SafeBrowse from an administrator shell or with Run as administrator. That increased the impact of a host/native-integration compromise. This finding did not establish that WebView2's renderer sandbox was disabled or demonstrate an exploit. Microsoft recommends a standard-integrity WebView2 host and separation of any work that truly requires elevation. [WebView2 security guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security).

**Remediation applied:** startup reads the current process token's `TokenElevation` and `TokenIntegrityLevel`. It rejects either an elevated token or an integrity RID at or above high, and fails closed if the token cannot be queried. The guard runs after read-only argument/help processing and before worker dispatch, the session mutex, clipboard clearing, desktop acquisition/creation, or profile/UI construction. The error tells the user to launch normally without Run as administrator. This prevents elevated hosting; it does not create a restricted token or sandbox the ordinary user host. [TOKEN_ELEVATION](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-token_elevation), [mandatory integrity control](https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control).

**Verification and remaining test surface:** pure tests cover the documented RID boundaries and refusal when either elevation or high integrity is present; a native test reads the current token and checks the classification. These do not require or trigger UAC elevation. The guard's position before mutable startup is source-verified. Administrator-launch and failure-path behavior still belongs in disposable-machine release validation.

### SB-04: recovery is process-exit monitoring

**Locations:** `src/desktop/recovery.rs:38`, `src/desktop/recovery.rs:87`; `src/main.rs:223`.

The watchdog is a thread in the supervisor. It waits on the worker's process handle and restores the original desktop when the worker exits. A live but unresponsive worker does not signal that handle. Force-terminating the supervisor also terminates its watchdog and does not execute Rust drop guards. The new kill-on-close job improves worker lifetime enforcement when supervisor handles close, but does not provide an independent desktop-restoration process or a responsiveness signal.

**Precondition and impact:** a UI/runtime deadlock, supervisor crash/forced termination, or local interference can defeat automatic recovery. The supervisor's shortcut may still work for a hung worker while the supervisor remains responsive; Windows recovery mechanisms remain separate. The finding is a gap in automatic availability, not proof that Windows itself becomes inaccessible.

**Mitigation:** add an authenticated, bounded responsiveness signal and define a safe recovery policy. Consider an independent recovery process if surviving supervisor failure is a requirement; do not describe an in-process RAII guard as covering OS termination. Test recovery without depending on the browser event loop, and retain a documented Windows-level escape route.

**Test surface:** worker event-loop hang, abrupt worker exit, and abrupt supervisor exit on a disposable machine. These fault tests were not performed in this review.

### SB-05: website messaging transport remained enabled

**Locations:** `src/browser/security.rs::harden_content_view`, `src/ui/kiosk.rs::build_content_view`, and `src/browser/controller.rs::create_webview`; dependency `wry-0.56.1/src/webview2/mod.rs:481`, `:942`.

Wry always registers `WebMessageReceived` and injects `window.ipc.postMessage`, including when `.with_ipc_handler` is omitted. With no application handler it returns early, so the reviewed code did not expose application commands to websites. Nevertheless, website JavaScript could still invoke the renderer-to-host messaging transport; the previous documentation's stronger statement that messaging was disabled was inaccurate. No command-execution or measured denial-of-service exploit was demonstrated.

**Remediation applied:** both website builders use a shared policy that explicitly sets `IsWebMessageEnabled(false)` and `AreHostObjectsAllowed(false)` before any website navigation or popup attachment. Native setting failures abort view creation. Trusted control views retain their separate messaging policy. Disabling unused bridges also follows Microsoft's guidance. [WebView2 security guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security).

**Verification:** the `assert_native_content_security_settings` fixture in `src/ui/kiosk.rs` builds the actual production content view and reads both native properties back as false. It also reads the SmartScreen requirement back as true. The same parent test creates a trusted view, confirms messaging remains enabled, and receives its actual UI commands. Run:

```text
cargo test --lib ui::kiosk::tests::bundled_shell_surfaces_load_and_deliver_commands_inside_native_webviews -- --exact --nocapture
```

The fixture uses temporary local data and does not grant permissions to a real site. It checks the installed native runtime, not every supported Windows/WebView2 combination. Current execution results are maintained in [VALIDATION.md](../VALIDATION.md).

### RG-01: release and update assurance is unfinished

**Locations:** `Cargo.toml:16`, `Cargo.lock`, `README.md:79`; repository release/build configuration.

The lockfile pins the resolved Rust dependency graph, and the project uses Microsoft's installed WebView2 runtime. The inspected repository has no checked-in release CI, signing/attestation workflow, dependency-advisory automation, application updater, or enforced runtime security-version policy. This is a release-readiness observation, not evidence that a specific dependency is vulnerable or that an existing published binary has been tampered with. No current advisory-database scan or binary-signature verification was performed.

**Impact:** community users need a way to identify authentic builds and receive urgent application fixes. Runtime servicing alone does not update SafeBrowse's Rust host, Wry code, or permission logic. Evergreen runtime updates also require application restarts before a running session uses the new version. [WebView2 distribution guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution).

**Mitigation:** establish locked, reviewable release builds; authenticated release artifacts and provenance; dependency/advisory checks; an SBOM; security ownership and response procedures; and a documented supported Windows/runtime matrix. Decide how users learn that an application or runtime restart/update is required. Add a signed update protocol only after its own trust, rollback, and failure behavior are designed and reviewed.

## Controls and limits checked in source

| Area | Observed control | Limit of this review |
| --- | --- | --- |
| Trusted UI / IPC | `src/ui/trusted.rs:26` permits only the exact bundled data URL and `about:blank`; `src/ui/assets.rs:13` supplies a restrictive local CSP; `src/ui/assets.rs:39` escapes presentation JSON. `src/ui/kiosk.rs:122` assigns IPC surface identity natively and bounds message size. | No remote-to-trusted command execution path was established. This is not comprehensive XSS fuzzing or a proof that future templates cannot introduce one. |
| Website permissions | `src/browser/requests.rs:506` denies initially, requires nonpersistent native decisions, derives origin from engine data, and rejects cross-origin embedded capability requests. `src/browser/permissions.rs:261` canonicalizes exact scheme/host/port origins; persisted schema and sizes are checked. | Permission changes govern future requests; reload/close is needed for access already in use. Same-user tampering with configuration files is outside the established boundary. |
| Popups / stale requests | `src/browser/requests.rs:557` marks the native request handled and requires the initiating frame's identity; unknown frame identity fails closed. Deferrals are bounded and cancelled on navigation/closure. `src/ui/kiosk/site_requests.rs:206` verifies the trusted prompt source and current request ID. | No real banking OAuth, nested-frame, or comprehensive popup compatibility testing was performed by this review. |
| Screen capture | `src/security/capture.rs:24` sets and reads back `WDA_EXCLUDEFROMCAPTURE`. The explicit CLI override is not persisted; the trusted red warning gates use at `src/ui/kiosk.rs:981`, with lasting recording indicators. | Display affinity is a platform capture-exclusion feature, not a guarantee against every capture mechanism or a compromised machine. The intended debug flag is not classified as a vulnerability. [Microsoft documentation](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity). |
| Virtual keyboard | `src/keyboard/osk.rs:87` JSON-encodes key text. `src/keyboard/input.js:21` writes to the editable page DOM and dispatches input events, without a generic native host-object API. | It is not a trusted secret-entry path against malicious page scripts, renderer compromise, or same-user process inspection. The receiving website can observe values entered into its own fields. |
| Browser/runtime | `src/config.rs:55` preserves WebView2 sandbox defaults; no `--no-sandbox` switch was found in application configuration. Downloads are denied by `src/ui/kiosk.rs:188`. | Host containment, runtime-policy/environment overrides, native dialogs, accessibility/process attacks, malicious drivers, and OS compromise were not established as defended boundaries. |
| Reputation checking | `src/browser/security.rs::harden_content_view` explicitly sets `ICoreWebView2Settings8::IsReputationCheckingRequired` to true and reads it back before website navigation or popup attachment. Both website builders call this shared policy. An unsupported interface, setting error, or false readback aborts view creation. | This verifies the application setting only. Microsoft documents that Windows can disable SmartScreen while preserving a true WebView2 value. SafeBrowse has no separate reputation service and does not override that policy. [Microsoft documentation](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2settings.isreputationcheckingrequired). |
| Profiles | `src/browser/profile.rs` creates unique temporary directories and removes them after normal shutdown with bounded retries. | Crash/forced-termination remnants and secure erasure remain unresolved by this mechanism, as already documented. |

## Recommended order of work

1. Define and verify the local-adversary model, especially the same-user desktop/process access that the launch remediations do not remove. Review the new authentication and lifetime protocol independently.
2. Expand isolated-machine adversarial tests for desktop access/creation races, process integrity, worker replay and lifecycle faults, recovery, and capture methods. Preserve the current origin/IPC/permission and native SmartScreen-setting regression tests.
3. Establish an authenticated maintenance and distribution process, then obtain an independent architecture and implementation assessment before positioning the app for sensitive financial use.

No claim about defeating active local malware, protecting credentials from a hostile website, or matching Safepay should be inferred from UI polish, passing unit tests, or capture-exclusion success alone.
