# Release candidates and provenance

For the current-user unsigned installer, see [INSTALLER.md](INSTALLER.md). Its local working-tree builder is separate from the clean committed ZIP candidate process below; GitHub setup is not required for a local installer build.

The repository supplies repeatable commands and a GitHub candidate workflow. Creating these files does not create a public release, a signature, or an attestation. No workflow has been run merely by adding it to a local checkout. A successful build and attestation identify an artifact's origin; they do not certify the application as secure or prove resistance to a compromised Windows account.

## Prepare the source

Commit the complete reviewed application, tests, assets, `Cargo.lock`, build script, toolchain file, and release tooling. `scripts/New-ReleaseCandidate.ps1` refuses tracked changes, untracked files, missing required tracked inputs, and ignored files in application input directories. The current work should first be reviewed and committed by its owner. The script never creates commits.

Rust is pinned to **1.97.0**, with `clippy`, `rustfmt`, and `x86_64-pc-windows-msvc`, in `rust-toolchain.toml`. Windows release and native test hosts also need the Microsoft C++ Build Tools, Windows SDK, and a working WebView2 Evergreen Runtime. The native tests create hidden windows; the CI host must support Windows desktop and WebView2 APIs. A hosted-runner failure is a failed validation, not permission to silently skip those tests. Node **24.18.0** runs the JavaScript fixtures without third-party JavaScript dependencies. Review these pins regularly and update them through an explicit change.

Run from a PowerShell 7 session at the repository root:

```powershell
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node --test tests/keyboard_dom_tests.cjs tests/website_print_guard_tests.cjs
cargo install cargo-audit --version 0.22.2 --locked
./scripts/Test-WindowsDependencies.ps1
./scripts/Test-ReleaseTooling.ps1
./scripts/New-ReleaseCandidate.ps1 -ValidateOnly
./scripts/New-ReleaseCandidate.ps1
```

The dependency checker resolves the locked Windows graph, then matches RustSec findings by package name, version, and source. Findings in that graph fail the check, including maintenance/unsoundness warnings. Other-platform findings remain visible in `target/windows-dependency-audit.json` but do not falsely block the Windows product. An audit failure or unusable report fails closed. This check does not audit the separately installed WebView2 browser engine, downloaded content, or application logic. The advisory database revision is recorded in the report; review freshness and newly disclosed issues before distribution.

The release-tooling fixtures create a separate repository below `target/release-tooling-tests` and mock compiler/audit outputs. They verify clean-source rejection, target filtering, export/packaging and checksum behavior; their fake executable must never be distributed. They do not compile or publish the product, change the surrounding repository's Git state, or replace the real checks above.

## Inspect a local candidate

Each build gets a fresh `target/release-candidates/<revision-and-random-id>` directory. The script exports the clean committed revision with `git archive`, extracts it, and compiles that source into a fresh target directory. It avoids reusing a previously built application binary and prevents ignored workspace source files from becoming build inputs. It also rejects common compiler/wrapper environment overrides. No existing candidate is overwritten or removed.

The `distribution` subdirectory contains:

- A ZIP with `safebrowse.exe`, `LICENSE`, `README.md`, `SECURITY.md`, the documentation directory, and `provenance.json`.
- `SHA256SUMS.txt`, containing the distribution ZIP's SHA-256 digest.
- A separate copy of `provenance.json`, recording the source revision, source archive and lockfile hashes, target, compiler/Cargo identities, build time, Windows version, executable hash, and observed Authenticode status.

The source export and build tree are retained next to the distribution files for inspection. Do not distribute that entire working directory. Paths to local source files in developer review documents are contextual references; users should use the committed repository revision to inspect source.

A local build still trusts its host, Rust toolchain, Cargo configuration/cache, SDK, dependency sources, and native upstream libraries. This process does not establish byte-for-byte reproducible builds or prove that the build machine was uncompromised. `git archive` respects committed export attributes. Review such attributes and all build-input changes together with the candidate. Do not copy a separately signed executable over an already hashed or attested file; any later signing step requires new packaging, hashes, and attestations.

Before accepting a candidate, perform and record the applicable native interaction, capture, recovery, profile cleanup, runtime-version and permission/download/printing checks in `VALIDATION.md`. Use disposable test content. Preserve the exact commit, candidate digest, Windows build, WebView2 version and results in the release record. The candidate script itself records build success only; it does not claim those checks were performed.

## GitHub verification and attestation

After pushing the reviewed repository, **Windows verification** runs on pushes and pull requests with read-only repository permissions. Dependabot proposes Cargo and GitHub Action updates weekly; it does not merge them automatically. Action references are pinned to full commits. The initial pins were resolved from each official action repository's GitHub release/tag API on 4 September 2026.

Manually dispatch **Build an attested Windows candidate** for the intended committed revision. It runs the same verification workflow, builds a clean source export in a separate Windows job, and uploads a review artifact. A final job downloads only that run's finished artifact and creates a GitHub build-provenance attestation; this job has OIDC and attestation permissions but never checks out or executes project code. The workflow does not create a GitHub Release, push commits, upload an installer elsewhere, or use an Authenticode certificate. Protect the publication branch and review changes to workflows, release scripts and dependency pins before dispatching it.

GitHub's artifact expiration applies to the uploaded candidate. Successful attestation must be checked in the actual workflow run; the local provenance file intentionally cannot promise that a later job succeeded. Verify the archive and JSON from the downloaded artifact using the real owning repository:

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath ./safebrowse-VERSION-windows-x64-REVISION.zip
gh attestation verify ./safebrowse-VERSION-windows-x64-REVISION.zip --repo OWNER/REPOSITORY
```

Compare the digest with `SHA256SUMS.txt`, and inspect the attestation's source repository, source revision, workflow identity and run. Substituting an attacker-controlled repository in verification defeats the identity check. A checksum alone detects corruption relative to the supplied checksum; it does not authenticate the publisher.

GitHub attestations and Windows Authenticode solve different distribution problems. This workflow creates the former only after a successful authorized GitHub run. Windows executables remain unsigned unless a separate reviewed Authenticode process and real certificate are configured. The provenance records that status rather than inventing a publisher signature. No signing credentials are required or stored by this tooling.

Official references: [GitHub build attestations](https://docs.github.com/en/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds), [actions/attest](https://github.com/actions/attest), [GitHub CLI attestation verification](https://cli.github.com/manual/gh_attestation_verify), [RustSec cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit), and [Microsoft WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution).
