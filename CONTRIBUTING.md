# Contributing to SafeBrowse

Thanks for helping make SafeBrowse more reliable. Useful contributions include reproducible bug reports, accessibility improvements, clearer documentation, Windows compatibility checks and small, reviewable fixes.

SafeBrowse is a public preview. Please read [SECURITY.md](SECURITY.md) before changing capture, browser permissions, desktop switching, printing, downloads or file cleanup. Describe the protection a change actually establishes and keep unverified claims out of documentation and user-facing text.

## Reporting a problem

Use the [issue forms](https://github.com/itsdanielyc/SafeBrowse/issues/new/choose) for ordinary bugs and feature requests. Search existing issues first. A useful report includes:

- SafeBrowse version or commit, Windows version/build and WebView2 Runtime version.
- Exact steps, expected behavior, actual behavior and error text, using disposable test content.
- Relevant launch options, monitor layout/scaling and recording software plus capture mode.

Remove account names, credentials, tokens, personal paths and browsing data from attachments. A synthetic test page is preferable to a screenshot of a real account. Do not run tests on another person's computer or services without permission.

**Security issues belong in [private vulnerability reports](https://github.com/itsdanielyc/SafeBrowse/security/advisories/new), not public bug reports.** This includes a suspected capture-protection bypass. Follow the fallback contact process in [SECURITY.md](SECURITY.md) if private reporting is unavailable.

## Development setup

Use Windows x64, the Rust MSVC toolchain pinned in [rust-toolchain.toml](rust-toolchain.toml), Microsoft C++ Build Tools with the Windows SDK, a supported WebView2 Evergreen Runtime, PowerShell 7 and Node.js for the JavaScript fixtures. CI records the supported tool versions in [.github/workflows/ci.yml](.github/workflows/ci.yml).

```powershell
git clone https://github.com/itsdanielyc/SafeBrowse.git
cd SafeBrowse
cargo build --locked
```

For interface work on the current desktop, use `--windowed`. Capture protection remains enabled unless you explicitly add `--allow-screen-recording`; that override shows a warning and must only be used with non-sensitive test content. See the [usage guide](docs/USAGE.md#recordable-development-mode).

## Before a pull request

Keep the change focused. Explain the user-visible problem, the resulting behavior, validation performed and any remaining platform limitations. Discuss large changes in an issue before implementing them. Preserve existing security defaults and fail closed when a required protection cannot be established.

Run relevant checks from the repository root:

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node --test tests/keyboard_dom_tests.cjs tests/website_print_guard_tests.cjs
```

Native WebView2 tests need an interactive Windows session and an installed runtime. The default test suite does not switch your active desktop or clear your clipboard. Tests that deliberately perform those actions require explicit opt-in. Use a disposable Windows account or VM for installer cleanup, failure injection and capture experiments.

For release-tooling changes, run `./scripts/Test-ReleaseTooling.ps1`. For installer changes, follow the fixture and manual checks in [docs/INSTALLER.md](docs/INSTALLER.md). Dependency changes should include the Windows-target audit described in [docs/RELEASING.md](docs/RELEASING.md).

Add regression coverage when it exercises the failure or behavior being changed. Manual evidence should identify Windows/runtime versions and the exact scenario; passing a test is not a general security guarantee. Do not check in generated executables, personal browser data, credentials, build directories or large working media projects.

## Style and review

Follow the surrounding Rust and web UI conventions. Prefer descriptive names, explicit types, small functions and clear ownership. Comments should explain why a non-obvious rule exists. Avoid weakening browser isolation, introducing privileged website bridges, following untrusted cleanup paths or silently retrying submissions.

Update user documentation when behavior changes. Mention known limitations honestly and provide accessible labels for interactive controls. Keep interface language plain and actionable.

Contributions are reviewed under the project's [MIT License](LICENSE); dependencies retain their own terms. Be considerate and keep discussion focused on the work.
