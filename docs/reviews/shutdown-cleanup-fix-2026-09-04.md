# Normal-close cleanup correction — 4 September 2026

A user opening and normally closing `target/security-remediation/release/safebrowse.exe` received a temporary-profile cleanup error ending in Windows error 2, followed by the supervisor's exit-code-1 dialog. A deterministic regression reproduced the same error before the correction. The screenshot does not identify the individual child path, so it establishes the matching failure class rather than a trace of the original filesystem race.

## Change

Directory enumeration can return a cache entry that WebView2 removes before SafeBrowse opens it for deletion. `delete_children` previously propagated that missing-child error as a permanent cleanup failure. It now delegates each enumerated child to `delete_child`, which tolerates `NotFound` only at that child's open/check boundary. Cleanup still requires deletion of the remaining children, the session marker and the pinned session directory. Missing roots or markers, access failures, junctions, hard links, and exhausted cleanup budgets remain errors under their existing rules. No broad missing-file exception was added to whole-profile cleanup.

The native top-level dialog caption is now exactly **SafeBrowse error**, because the reporter also receives browsing and shutdown failures. Error bodies and console/dialog routing retain their existing behavior. The [message catalogue](error-message-catalogue-2026-09-04.md) reflects the changed caption.

WebView2 teardown can outlive closing its controls; Microsoft's [user-data-folder guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder) discusses waiting for browser processes to release files. This correction preserves SafeBrowse's existing bounded retries for still-locked data and handles a child disappearing during cleanup. It does not add forced browser termination or change profile ownership checks.

## Verification

- Before the behavioral correction, `vanished_enumerated_children_do_not_prevent_complete_profile_cleanup` failed with `Os { code: 2, kind: NotFound, message: "The system cannot find the file specified." }`.
- Afterward, all nine focused profile tests passed. New deterministic tests cover both a file and directory disappearing after enumeration, and an enumerated entry being replaced by a hard link to an outside sentinel. Existing tests still enforce lock handling, ownership records, bounded work, and refusal of junctions and hard links.
- A new native regression ran three hidden production WebView2 sessions on loopback pages with disposable profiles and dummy local storage. It checked complete profile removal, normal browser-process exit, no false engine-failure notification after teardown, and no repeated document request.
- `cargo test --locked --offline`: **137 passed, 4 ignored, 0 failed**. The ignored interactive/destructive tests were not invoked.
- `cargo fmt --check` and `cargo clippy --all-targets --locked --offline -- -D warnings` passed.
- `cargo build --release --locked --offline --target-dir target/security-remediation` passed. The resulting executable's `--help` check passed without starting a browsing session.

The full visible isolated-desktop application was not relaunched for this correction. Hidden native fixtures do not replace a manual open/close check of that complete UI. No Windows settings, clipboard contents, capture flags, persistent browsing data, or unrelated executable paths were changed. The old error-dialog process had already exited by the guarded close check, so no process-close request was needed.

## Local build

Updated executable: `target/security-remediation/release/safebrowse.exe` (2,486,784 bytes).

SHA-256: `a19ca271e00caf70e40efd93fa1be11185aa19607e0cae13f3aa34565212c8a1`.

The [new source manifest](shutdown-cleanup-fix-2026-09-04.sha256) records the current inputs after this correction. Earlier review/remediation manifests remain historical snapshots and do not describe the new source or executable. This remains an unsigned local working-tree build; no release was published.
