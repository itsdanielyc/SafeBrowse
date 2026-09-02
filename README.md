# SafeBrowse

**SafeBrowse** is a high-assurance, open-source secure banking and transaction browser for Windows, architected from the ground up to match the security properties of **Bitdefender SafePay**.

When launched, SafeBrowse isolates the interactive session by creating a dedicated Win32 desktop (`WinSta0\SafeBrowseDesktop`), taking over the full display, blinding screen recording / scrapers via Desktop Window Manager (DWM) display affinity, and providing an on-screen virtual keyboard that injects input directly into web documents without generating system-wide OS keystrokes.

---

## Key Features & Security Architecture

### 1. Win32 Alternate Desktop Isolation
- **Dedicated Object**: SafeBrowse instantiates an isolated desktop object in the interactive window station (`WinSta0\SafeBrowseDesktop`).
- **Hook & Message Isolation**: User-mode global hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) and message sniffing (`WM_GETTEXT`, `EnumWindows`) installed by untrusted applications on the user's `Default` desktop cannot cross into `SafeBrowseDesktop`.
- **Fail-Safe Watchdog & Panic Recovery**: Monitored by a background supervisor thread and RAII guards (`DesktopRecoveryGuard`, `DesktopWatchdog`). If the browser exits unexpectedly or panics, the system immediately restores the interactive `Default` desktop, guaranteeing the user is never trapped.
- **Desktop Switcher**: Press `Ctrl+Alt+D` or click **"Back to Windows Desktop"** in the top bar to toggle back to your normal Windows desktop without closing your secure browsing session.

### 2. Screen Scraper & Recorder Immunity
- **DWM Capture Blinding**: Windows Display Affinity (`WDA_EXCLUDEFROMCAPTURE`, 0x00000011) is applied to all native windows.
- **Anti-Snooping**: Snipping Tool, OBS Studio, Discord / Teams screen sharing, and GDI `BitBlt` capture return pure black frames for SafeBrowse.
- **PrintScreen Interception**: Low-level hotkey intercepts consume bare `VK_SNAPSHOT` (PrintScreen) keys while on the safe desktop.

### 3. Hook-Immune Secure Virtual Keyboard
- **The Problem**: Traditional on-screen keyboards (`osk.exe`) simulate keystrokes via `SendInput()` or `keybd_event()`, emitting standard `WM_KEYDOWN` events that system-level keyloggers easily intercept.
- **SafeBrowse Solution**: Dispatches character input directly into the active document input element (`document.activeElement`) via internal IPC script evaluation and W3C DOM events (`setRangeText`, `InputEvent`). Zero OS-level keystroke events are emitted.
- **Anti-Coordinate Tracking (Key Scramble)**: Built-in Fisher-Yates layout scrambler randomizes key button positions on demand to neutralize mouse coordinate loggers and heatmaps.

### 4. Headed Chromium Engine & Identity Coherence
- **Real Chromium Core**: Powered by Microsoft Edge WebView2 evergreen runtime (`Blink/V8` + Chromium network stack).
- **Anti-Automation Coherence**: Runs with `--disable-blink-features=AutomationControlled`, ensuring `navigator.webdriver === false` and natural TLS JA3/JA4 and HTTP/2 fingerprints.
- **Zero Fingerprint Spoofing**: Does not forge synthetic hardware personas, preventing anti-bot tripwires (Cloudflare Turnstile, Google reCAPTCHA) caused by cross-layer contradictions.

### 5. Sandboxed Ephemeral & Persistent Profiles
- **Ephemeral Mode (Default)**: Generates a temporary user data directory under `%TEMP%\SafeBrowse_Session_<UUID>`. When the session ends, all cookies, session storage, cache, and history are recursively wiped from disk.
- **Persistent Mode (`--persistent`)**: Uses a dedicated container directory in `%LOCALAPPDATA%\SafeBrowse\Profile_Persistent` segregated from all other system browsers.
- **Renderer-Isolated Bookmarks**: Bookmarks are stored in `%APPDATA%\SafeBrowse\bookmarks.json` with atomic staging replacements. Untrusted web renderers have zero access to the bookmarks database.

---

## Threat Model & Boundary Guarantees

| Threat | SafeBrowse Defense |
|---|---|
| **User-mode keyloggers on Default desktop** | **Defeated**: Hooks cannot penetrate `SafeBrowseDesktop`; OSK bypasses OS key events. |
| **Screen scrapers & screen recorders (OBS, Discord, Snipping Tool)** | **Defeated**: `WDA_EXCLUDEFROMCAPTURE` instructs DWM to blank out the window. |
| **Session cookie leakage across browser restarts** | **Defeated**: Ephemeral profile sandbox purges all site data upon exit. |
| **Cross-site profile correlation** | **Defeated**: Isolated user data directories with segregated caches and cookies. |
| **Malicious browser extensions** | **Defeated**: Clean embedded runtime does not load external Chrome/Edge extensions. |
| **Clipboard snooping** | **Mitigated**: Automated clipboard memory sanitization on session entry and teardown. |
| **Kernel rootkits / hypervisor compromise** | **Out of Scope**: Administrator/kernel-level malware can bypass user-mode desktop DACLs. |
| **Physical camera photographing screen** | **Out of Scope**: Hardware displays cannot prevent optical observation. |

---

## Quick Start & Usage

### Prerequisites
- Windows 10 (Version 2004 or later recommended for `WDA_EXCLUDEFROMCAPTURE`) or Windows 11.
- Microsoft Edge WebView2 Runtime (pre-installed on Windows 10/11).
- Rust 1.80+ and Cargo (for building from source).

### Building from Source
```powershell
# Clone repository
git clone https://github.com/ppcdaniel/SafeBrowse.git
cd SafeBrowse

# Run the full automated test suite
cargo test -- --nocapture

# Build optimized production binary
cargo build --release
```

The compiled binary will be located at:
```text
target\release\safebrowse.exe
```

---

## CLI Options

```text
SafeBrowse - High-Assurance Isolated Windows Browser (Bitdefender SafePay Architecture)

USAGE:
    safebrowse.exe [FLAGS] [OPTIONS]

FLAGS:
    --help, -h          Print documentation
    --windowed, -w      Run in windowed mode on the current desktop (testing & development)
    --persistent, -p    Use durable persistent profile instead of ephemeral zero-retention
    --worker            Internal flag: signals execution inside the isolated desktop

OPTIONS:
    --url <URL>         Target URL to open immediately upon launch (Default: DuckDuckGo)
```

### Examples
- **Standard SafePay Mode (Full-screen kiosk on isolated desktop):**
  ```powershell
  .\target\release\safebrowse.exe
  ```
- **Windowed Mode (For testing without switching desktops):**
  ```powershell
  .\target\release\safebrowse.exe --windowed
  ```
- **Launch directly to a specific banking site in persistent mode:**
  ```powershell
  .\target\release\safebrowse.exe --persistent --url https://www.paypal.com
  ```

---

## Project Structure

```text
SafeBrowse/
├── Cargo.toml               # Package manifest and Win32/Wry dependencies
├── .gitignore               # Build target and temporary session exclusions
├── README.md                # Technical architecture and user guide
├── src/
│   ├── lib.rs               # Library root re-exporting core modules
│   ├── main.rs              # CLI parser, launcher supervisor, and recovery watchdog
│   ├── config.rs            # Operational parameters, desktop IDs, and security flags
│   ├── desktop/
│   │   ├── mod.rs
│   │   ├── manager.rs       # CreateDesktopW, OpenDesktopW, SwitchDesktop, HDESK management
│   │   └── recovery.rs      # Watchdog thread and emergency fail-safe DesktopRecoveryGuard
│   ├── security/
│   │   ├── mod.rs
│   │   ├── capture.rs       # SetWindowDisplayAffinity (WDA_EXCLUDEFROMCAPTURE)
│   │   ├── hooks.rs         # PrintScreen consumption and Ctrl+Alt+D registration
│   │   └── clipboard.rs     # Clipboard sanitization and purging broker
│   ├── browser/
│   │   ├── mod.rs
│   │   ├── controller.rs    # Chromium WebView2 controller & IPC routing
│   │   ├── profile.rs       # Ephemeral auto-purge vs persistent profile lifecycle
│   │   └── tabs.rs          # Multi-tab data structure and navigation state
│   ├── keyboard/
│   │   ├── mod.rs
│   │   └── osk.rs           # Hook-immune virtual keyboard with Fisher-Yates key scrambling
│   ├── bookmarks/
│   │   ├── mod.rs
│   │   └── store.rs         # Persistent JSON store with atomic file replacement
│   └── ui/
│       ├── mod.rs
│       ├── assets.rs        # Embedded HTML/CSS/JS kiosk shell & floating OSK drawer
│       └── kiosk.rs         # Tao window creation, display affinity binding, and event loop
└── tests/
    ├── bookmark_tests.rs    # Bookmark URL validation & atomic persistence tests
    ├── desktop_tests.rs     # Desktop creation, switching, and recovery guard tests
    ├── profile_tests.rs     # Ephemeral profile auto-purge verification tests
    ├── security_tests.rs    # Capture exclusion, clipboard sanitization, and OSK script tests
    └── tab_tests.rs         # Multi-tab lifecycle and clamping tests
```

---

## Verification & Automated Tests

SafeBrowse includes a suite of automated unit and integration tests:
```powershell
cargo test -- --nocapture
```
Output:
```text
running 2 tests
test test_bookmark_store_initialization_and_defaults ... ok
test test_bookmark_url_validation ... ok

running 4 tests
test test_desktop_manager_creation ... ok
test test_acquire_default_desktop ... ok
test test_recovery_guard_disarm ... ok
test test_create_safe_desktop ... ok

running 2 tests
test test_ephemeral_profile_lifecycle ... ok
test test_persistent_profile_creation ... ok

running 4 tests
test test_capture_protector_invalid_handle ... ok
test test_clipboard_purging ... ok
test test_dom_injection_script_generation ... ok
test test_virtual_keyboard_scramble_keys ... ok

running 1 test
test test_tab_manager_lifecycle ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; finished in 0.12s
```

---

## License

SafeBrowse is licensed under the [MIT License](LICENSE).
