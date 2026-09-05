# SafeBrowse security and engineering review — 4 September 2026

**Follow-up:** the user subsequently authorized code fixes. The [remediation record](security-remediation-2026-09-04.md) describes those changes and their remaining limits. This review and its SHA-256 manifest describe the earlier source snapshot; findings below must be read in that historical context.

**Verdict:** No evidence of an intentional backdoor or covert data collection was found in the reviewed application source. That is not proof that every dependency, existing executable, installed runtime, or the computer is uncompromised. SafeBrowse implements useful parts of a Safepay-style browsing environment, but neither its implementation nor the available tests establish equivalent security or production reliability, even excluding the deliberately omitted password manager, VPN, and Bitdefender website reputation service.

**Reviewed state:** the local working tree based on `c288c11ad4292503a9fe6ef9b5aeaf9a7d401fce`, including modified and untracked files. This is substantially different from the committed revision. `Cargo.lock`, `SECURITY.md`, the launch-authentication implementation, and browser security settings are among the untracked files. This review must not be represented as approval of that commit or of an existing release executable. The accompanying [SHA-256 manifest](security-review-2026-09-04.sha256) identifies 98 application, build, test, documentation, and asset files at review completion.

**Method:** separate source reviews of Windows/session protections, browser boundaries, trusted UI/input, and build/dependency handling; verification against Microsoft and Bitdefender primary sources; fresh local Rust/JavaScript checks; and a benign, disposable Windows access-rights probe. GitNexus was consulted for repository discovery, but SafeBrowse was not indexed, so source tracing used direct file reads and reference searches. Earlier review documents were treated as claims to verify, not as evidence that their stated fixes were present.

No application source was changed during this review. No banking account was used, no native print job was submitted, and no real browser session was subjected to memory inspection, injection, input logging, or forced termination.

## Findings requiring action

### R1 — P2: ordinary website print suppression is missing from the production builder

**High confidence; source-confirmed integration defect.** `src/ui/kiosk.rs:220` installs the input-focus initialization script in `build_content_view`, but does not install `WEBSITE_PRINT_GUARD`. Normal tabs reach this builder through `show_active_tab` at line 766, and approved popups reach it through `src/ui/kiosk/site_requests.rs:175`.

The print guard is instead installed in `build_trusted_view` at `src/ui/kiosk.rs:160`, which creates the application controls, and in `BrowserController::create_webview` at `src/browser/controller.rs:105`. Reference searches found no production callers of that controller.

Consequently, the application does not replace ordinary website `window.print()` on its actual browsing path. The saved Printing setting gates the host toolbar and shortcut at `src/ui/kiosk.rs:1149`; it does not gate a website's native print function. This contradicts the ordinary-call suppression described in README/SECURITY and the statement in `docs/VALIDATION.md:97` that both website builders install the guard.

This is separate from the already documented limitation involving a popup's retained native print reference. The defect affects the normal installation of the wrapper itself. The standalone JavaScript tests and `print_wrapper_probe` construct/test the guard explicitly, so they do not prove that the production builder installs it.

**Impact:** a website retains a path into native printing UI despite the described suppression. Native printer/save dialogs have not been shown to inherit the main window's capture exclusion. This review did not invoke native printing or demonstrate a capture leak, automatic print submission, or automatic PDF creation.

**Correction:** install the guard in the actual website builder before navigation and popup handoff. Verify its property descriptor through that builder for normal pages, frames, and popups without invoking an unverified native print function. Consolidate or remove the unused browser-construction path after checking public-library compatibility. Continue describing the wrapper as best-effort suppression, not an engine-wide printing prohibition.

### R2 — P2: browser-engine failures are not recovered independently of host exit

**High confidence missing handling; failure effect derived from the platform contract.** `src/ui/kiosk.rs:361` subscribes to source, history, and navigation-completion events. The application does not subscribe to WebView2 `ProcessFailed` or browser-process-exit events. The watchdog at `src/desktop/recovery.rs:96` waits on the SafeBrowse worker process, not its browser engine's health.

If a WebView2 browser process exits while the Rust worker remains alive, the affected WebViews require recreation; an ordinary reload is not a replacement for closed browser objects. A failure in the trusted UI environment can also remove the controls needed to recover. Microsoft documents the required handling in its [process model](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model) and [process failure kinds](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2processfailedkind).

**Correction:** handle renderer failures and browser-environment failure separately, cancel outstanding permission/download decisions, and offer native recovery or safe exit even if the HTML controls are unavailable. Do not automatically repeat submissions, payments, or other non-idempotent page actions. Test worker exit, engine exit, and renderer unresponsiveness as distinct cases. No crash was injected during this review.

## Boundaries that prevent a broader security claim

### R3 — High significance for malware resistance: the host and desktop remain accessible to the same Windows user

`src/desktop/manager.rs:230` calls `CreateDesktopW` without an explicit security descriptor. `src/desktop/launch_auth.rs:311` uses ordinary `CreateProcessW` with the launching token and default process/thread security attributes. The inherited-handle authentication and lifetime job validate startup and supervision; they do not create a separate security principal or stop a sufficiently capable peer process accessing the host.

[Microsoft documents](https://learn.microsoft.com/en-us/windows/win32/winstation/desktop-security-and-access-rights) that default desktop ACLs come from the parent window station. Requesting `0x01ff` access for SafeBrowse's own handle does not specify what access other processes are denied.

A hidden standard-user test process created a disposable desktop with the same access mask and null security attributes. A second standard-user process successfully enumerated it, opened it with all specific desktop rights, and opened the **test parent process** with the requested memory-operation, memory-read/write, thread-creation, and handle-duplication rights. The probe only acquired and closed handles. It did not read memory, inject code, collect input, switch desktops, or access a real SafeBrowse process. The desktop was closed afterward. This confirms the default-access concern on this machine, not an executed exploit against SafeBrowse. Local evidence is saved in `target/security-review-2026-09-04/desktop-access-probe-observed.json`, with the inline fixture preserved as `desktop-access-probe.ps1` in the same directory; the saved script was not rerun.

The practical consequence is that display-affinity readback and randomized desktop names do not establish resistance to a process that can inspect or tamper with the browser host. This limitation is already honestly disclosed in `SECURITY.md`; it is an architectural scope gap, not evidence of malicious authorship.

**Engineering decision:** a useful first release can explicitly assume a clean, maintained Windows account. If the intended promise includes resistance to active same-user malware, first design and validate an appropriate principal/process/storage boundary and trusted broker. An ACL tweak applied indiscriminately to the same user, a UUID, or keyboard shuffling is not sufficient. No proposed architecture should promise protection against an already compromised kernel or administrator without a separate, justified threat model.

### R4 — Medium significance: runtime configuration and patch state are trusted

The worker inherits its environment (`src/desktop/launch_auth.rs:318`). Runtime discovery accepts any nonempty version string and discards the version (`src/browser/runtime.rs:15`, `:33`). The project does not verify the effective runtime path, profile override, debugging configuration, or security-switch overrides, and does not handle a new runtime version becoming available.

Microsoft documents that non-elevated WebView2 hosts honor supported environment and registry overrides, including additional browser arguments. Those mechanisms can enable debugging or alter security behavior. See [WebView2 security guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security) and [browser flags](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags). These are conditional risks if local configuration is changed, not evidence that the current session has such overrides.

**Correction:** define which overrides are supported, reject or clearly diagnose unsafe production configurations, record the selected runtime version/path, and establish a maintained runtime support policy and restart notice. Environment sanitization alone does not address registry overrides or same-user tampering. Preserve non-elevated hosting; running the browser as administrator is not a suitable remedy. Evergreen servicing of WebView2 also does not update SafeBrowse's own code.

### R5 — Medium significance: shutdown cleanup and desktop recovery are conditional

`src/browser/profile.rs:109` performs filesystem deletion with bounded retries; `src/ui/kiosk.rs:1555` releases the views before the normal explicit purge. These are sensible normal-exit controls. Crashes, process termination, power loss, and persistent file locks can leave profile data. There is no startup recovery procedure for abandoned session profiles. Downloads and printed output intentionally live outside this cleanup.

The watchdog detects worker termination, not a hung worker. Its emergency switch ignores the result (`src/desktop/recovery.rs:103`), and the recovery guard cannot execute after forced supervisor termination. The kill-on-close job reduces orphaned processes but cannot by itself restore the input desktop or execute profile cleanup. These limitations are already documented.

**Correction:** test failure paths with disposable sessions, provide recovery that does not depend on the failed UI loop, and make incomplete cleanup visible. If adding abandoned-profile cleanup, identify only application-owned, inactive session directories and handle reparse points/concurrency safely; never delete directories solely because their names match a prefix. Promise normal-exit deletion, not secure erasure or universal trace removal.

### R6 — Release blocker for a trusted binary distribution: provenance and maintenance are not established

The inspected executables at `target/release/safebrowse.exe` and `target/website-printing/release/safebrowse.exe` returned `NotSigned` from `Get-AuthenticodeSignature`; this does not mean they contain malware. They were not reproduced from this exact source snapshot, so their correspondence to the reviewed code is unknown. The repository has no checked-in release/signing/attestation workflow, CI policy, automatic dependency maintenance, or Rust toolchain pin. Important security files and the lockfile are not yet committed.

**Correction:** publish a complete reviewed source revision, build and test that exact revision in a controlled environment, publish authenticated artifacts and build provenance, and document how users receive application fixes. Authenticode is one Windows distribution option; source-build instructions and independently verifiable provenance also matter. A self-updater is optional, but a supported, authenticated update process is necessary for a maintained security tool. Define a private vulnerability-reporting contact and a patch/release owner.

## Positive evidence and absence-of-compromise assessment

- Website and popup views explicitly disable web messaging and host objects before navigation (`src/browser/security.rs:19`). Trusted controls and websites use separate contexts and profile subdirectories (`src/ui/kiosk.rs:544`).
- Bundled documents restrict navigation, use a restrictive content policy, encode data safely, and render remote titles/URLs as text. No application-owned covert upload endpoint, embedded credential collector, persistence installer, downloaded executable loader, or deliberately disabled sandbox/certificate validation was found in the reviewed application source. Ordinary website and WebView2/SmartScreen network traffic was not comprehensively captured or audited.
- The worker handshake checks the live parent, executable identity, worker PID, pipe ownership, and assigned desktop. The worker joins its lifetime job before execution. Direct worker arguments alone are insufficient. Elevated/high-integrity hosting is refused before session side effects.
- Capture exclusion is applied and read back while application windows are hidden (`src/security/capture.rs:26`, `src/ui/kiosk.rs:513`). The recording override is explicit, session-only, and gated by a warning. The user's successful recorder test is valid evidence for that tested capture path; it was not rerun here. Microsoft's [display-affinity contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity) limits the guarantee to supported capture mechanisms.
- Permission requests use native origin information, exact-origin policy, default denial, bounded deferrals, and cancellation on relevant navigation. Downloads use app-owned decisions, sanitized names and fresh destinations, with no automatic execution. Persistent permission choices, downloads, and bookmarks intentionally outlive temporary browsing data.
- The virtual keyboard sends text into the selected document without generating ordinary OS character keystrokes. That reduces one observation path. The receiving website can read its own fields, and neither browser memory nor an already compromised host is protected by that routing. Ordinary global-hook desktop scoping is described by [Microsoft](https://learn.microsoft.com/en-us/windows/win32/winmsg/about-hooks); it does not establish universal keylogger resistance.

The root build script only embeds bundled resources. The Windows-filtered dependency graph resolves 126 third-party packages from crates.io, without Git/path overrides. Dependency advisories and build-script inspection supplement source review; they do not constitute a line-by-line audit of all third-party Rust, native code, upstream precompiled WebView2 loader libraries, or Microsoft's runtime.

## Fresh verification

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo test --locked --offline` | 118 passed, 0 failed, 4 ignored |
| `node --test tests/keyboard_dom_tests.cjs tests/website_print_guard_tests.cjs` | 14 passed |
| Cargo audit against RustSec | No vulnerability entries; 11 unmaintained warnings and one unsoundness warning, all absent from the resolved Windows normal/build graph |
| Disposable desktop/process access-rights probe | Same-user requested access succeeded; no actual browser attacked |

Toolchain: Rust/Cargo 1.97.0, Windows x86_64 MSVC, Node 24.18.0. RustSec database commit: `5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5`, dated 2 September 2026. The ignored entries cover installed-runtime discovery, a subprocess fixture used by its parent test, interactive desktop switching, and destructive clipboard clearing. No ignored entry was explicitly enabled during this review.

Passing these tests does not establish that every advertised control is wired into the production path: R1 is a direct counterexample. No new release binary was built or certified, no extensive bank-compatibility matrix was run, and no same-build head-to-head Safepay attack testing occurred.

## Comparison with Safepay within the requested scope

The omitted password manager, VPN, and Bitdefender website detection are treated as deliberate exclusions, not implementation defects. SafeBrowse does request Microsoft's SmartScreen; that is a distinct runtime service.

| Retained capability | SafeBrowse evidence | Equivalence assessment |
| --- | --- | --- |
| Separate browsing desktop and independent session | Implemented with a Win32 desktop and separate profiles | Same broad design idea; insufficient evidence of equal local-malware resistance |
| Suppression of supported screen capture | Explicit affinity setting/readback plus user's successful recorder test | Working protection for tested mechanism; no all-recorder or driver guarantee |
| On-screen keyboard | Direct document input, optional movement/shuffling | Similar input-routing idea; not proof of equivalent resistance to keyloggers or process inspection |
| Browser containment and trusted controls | WebView2 sandbox defaults retained; remote bridge disabled; host ordinary user process | Useful separation; host/self-protection and effective-runtime boundary not established |
| Stable recovery and private-session lifecycle | Normal exit cleanup and supervised worker; R2/R5 remain | Reliability incomplete |
| Maintained, verifiable distribution | Local source and tests; R6 remains | Production delivery assurance incomplete |

Bitdefender's [current Safepay guide](https://www.bitdefender.com/consumer/support/answer/2051/) advertises desktop isolation, screenshot/keylogger resistance, and an independent browser. It does not disclose enough current implementation detail to prove technical parity or quantify which product is stronger. Those descriptions are vendor claims, not an independent benchmark.

A [historical discussion with Safepay's self-identified lead developer](https://community.bitdefender.com/en/discussion/37584/2nd-try-safepay-keyloggers-and-screenscrapers/p1) describes a desktop in the same logon session, direct browser input from the virtual keyboard, and protection supplied by the wider security product. This supports saying that SafeBrowse reproduces several real design concepts. It does not establish the exact architecture of today's Safepay, nor that adopting CEF or writing a driver would automatically achieve parity.

## Recommended next test surface

1. **Actual browser construction and native exits:** assert the print wrapper on real normal/popup/frame views; exercise TLS errors, external URI attempts, PDF/print and authentication dialogs with disposable data; record which top-level windows have capture exclusion. Do not infer protection of system dialogs from protection of the main window.
2. **Failure and cleanup:** independently terminate a test-owned renderer, engine, worker, and supervisor; test a hung UI and locked profile; require a usable escape path, cancelled stale decisions, honest cleanup status, and no automatic resubmission of transactions.
3. **Declared adversary boundary and release:** test desktop/process access and unsafe runtime overrides in a disposable lab, then build a signed or otherwise authenticated candidate from the exact reviewed revision and repeat the relevant controls across supported Windows/WebView2 versions.

Open-sourcing this as an explicitly experimental project is reasonable. Presenting the current state as a proven Safepay-equivalent banking security product is not supported by the evidence. The immediate work is to correct the production wiring, recover from engine failures, and make the security and release claims match tested boundaries.
