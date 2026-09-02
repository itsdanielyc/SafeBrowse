//! Embedded Assets & UI Shell for SafeBrowse
//!
//! Provides the Bitdefender SafePay-inspired visual interface:
//! - Full-Screen Desktop Shell (wave wallpaper, bottom taskbar with "Switch to Desktop" and system status)
//! - Browser Window Chrome (custom title bar, tab strip with "+" and Bookmarks/Settings, omnibox)
//! - Bookmarks Page (matching SafePay screenshot 3)
//! - Settings Page (matching SafePay screenshot 4)
//! - Secure Floating Virtual Keyboard (with key scrambling and DOM injection)
//! - Default Desktop Companion Dock Window (matching SafePay screenshot 1)

use crate::bookmarks::Bookmark;
use crate::browser::tabs::TabItem;

/// Generates the HTML for the full-screen desktop shell taking over SafeBrowseDesktop.
/// Matches Bitdefender SafePay desktop layout (screenshot 2):
/// - Deep blue midnight wallpaper with luminous cyan wave curves and glowing star nodes
/// - Bottom taskbar with `[🖥️ Switch to Desktop]`, middle running app item, and right system tray status.
pub fn generate_desktop_shell_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SafeBrowse Desktop Shell</title>
<style>
    * {
        margin: 0;
        padding: 0;
        box-sizing: border-box;
        user-select: none;
    }
    html, body {
        width: 100vw;
        height: 100vh;
        overflow: hidden;
        background: #030712;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        color: #f1f5f9;
    }
    #wallpaper-container {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: calc(100% - 46px);
        overflow: hidden;
        background: radial-gradient(circle at 22% 38%, #081a3d 0%, #040c1e 50%, #01040a 100%);
    }
    .wave-svg {
        position: absolute;
        width: 100%;
        height: 100%;
        pointer-events: none;
    }
    #bottom-taskbar {
        position: fixed;
        bottom: 0;
        left: 0;
        width: 100%;
        height: 46px;
        background: #090e18;
        border-top: 1px solid rgba(255, 255, 255, 0.12);
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 16px;
        z-index: 9999;
        box-shadow: 0 -4px 16px rgba(0,0,0,0.6);
    }
    .taskbar-left {
        display: flex;
        align-items: center;
        gap: 12px;
    }
    .btn-switch-desktop {
        display: flex;
        align-items: center;
        gap: 8px;
        background: rgba(14, 25, 48, 0.85);
        color: #38bdf8;
        border: 1px solid rgba(56, 189, 248, 0.45);
        padding: 6px 14px;
        border-radius: 6px;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.2s ease;
    }
    .btn-switch-desktop:hover {
        background: #38bdf8;
        color: #0b1120;
        box-shadow: 0 0 12px rgba(56, 189, 248, 0.4);
        transform: translateY(-1px);
    }
    .btn-switch-desktop:active {
        transform: translateY(0);
    }
    .taskbar-center {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .task-item {
        display: flex;
        align-items: center;
        gap: 8px;
        background: rgba(255, 255, 255, 0.08);
        border: 1px solid rgba(255, 255, 255, 0.15);
        padding: 6px 14px;
        border-radius: 6px;
        font-size: 12px;
        font-weight: 600;
        color: #e2e8f0;
        cursor: pointer;
        transition: background 0.15s;
    }
    .task-item:hover {
        background: rgba(255, 255, 255, 0.15);
    }
    .taskbar-right {
        display: flex;
        align-items: center;
        gap: 14px;
        font-size: 12px;
        color: #cbd5e1;
    }
    .status-badge {
        display: flex;
        align-items: center;
        gap: 6px;
        color: #38bdf8;
    }
    .shield-icon {
        color: #ef4444;
        font-size: 13px;
    }
    .separator {
        color: rgba(255, 255, 255, 0.2);
    }
</style>
</head>
<body>

<div id="wallpaper-container">
    <svg class="wave-svg" viewBox="0 0 1920 1080" preserveAspectRatio="none">
        <defs>
            <linearGradient id="orbitGlow" x1="0%" y1="0%" x2="100%" y2="100%">
                <stop offset="0%" stop-color="#38bdf8" stop-opacity="0.8"/>
                <stop offset="40%" stop-color="#0284c7" stop-opacity="0.5"/>
                <stop offset="80%" stop-color="#1e3a8a" stop-opacity="0.2"/>
                <stop offset="100%" stop-color="#020617" stop-opacity="0.0"/>
            </linearGradient>
            <linearGradient id="brightArc" x1="10%" y1="0%" x2="90%" y2="100%">
                <stop offset="0%" stop-color="#00ffff" stop-opacity="0.9"/>
                <stop offset="50%" stop-color="#0284c7" stop-opacity="0.4"/>
                <stop offset="100%" stop-color="#030712" stop-opacity="0.0"/>
            </linearGradient>
            <filter id="softGlow" x="-20%" y="-20%" width="140%" height="140%">
                <feGaussianBlur stdDeviation="3" result="blur" />
                <feMerge>
                    <feMergeNode in="blur"/>
                    <feMergeNode in="SourceGraphic"/>
                </feMerge>
            </filter>
        </defs>

        <!-- Dynamic SafePay Concentric Spiral Beziers matching Screenshot 2 -->
        <path d="M -150,550 C 250,-150 1100,100 2100,-50" fill="none" stroke="url(#orbitGlow)" stroke-width="2.5" filter="url(#softGlow)" />
        <path d="M -150,620 C 300,-80 1150,180 2100,20" fill="none" stroke="url(#brightArc)" stroke-width="1.8" />
        <path d="M -150,690 C 350,-10 1200,260 2100,90" fill="none" stroke="url(#orbitGlow)" stroke-width="2.2" />
        <path d="M -150,760 C 400,60 1250,340 2100,160" fill="none" stroke="url(#brightArc)" stroke-width="2.0" filter="url(#softGlow)" />
        <path d="M -150,830 C 450,130 1300,420 2100,230" fill="none" stroke="url(#orbitGlow)" stroke-width="1.5" />
        <path d="M -150,900 C 500,200 1350,500 2100,300" fill="none" stroke="url(#brightArc)" stroke-width="2.8" filter="url(#softGlow)" />
        <path d="M -150,970 C 550,270 1400,580 2100,370" fill="none" stroke="url(#orbitGlow)" stroke-width="1.6" />
        <path d="M -150,1040 C 600,340 1450,660 2100,440" fill="none" stroke="url(#orbitGlow)" stroke-width="2.0" />

        <!-- Glowing Star Nodes along the arcs -->
        <circle cx="280" cy="180" r="3" fill="#ffffff" filter="url(#softGlow)" />
        <circle cx="480" cy="110" r="2.5" fill="#38bdf8" />
        <circle cx="720" cy="140" r="3.5" fill="#00ffff" filter="url(#softGlow)" />
        <circle cx="980" cy="220" r="2" fill="#ffffff" />
        <circle cx="1240" cy="310" r="3" fill="#38bdf8" />
        <circle cx="1520" cy="380" r="2.5" fill="#00ffff" />
        <circle cx="1800" cy="420" r="3" fill="#ffffff" />
        <circle cx="340" cy="420" r="2.5" fill="#38bdf8" />
        <circle cx="590" cy="330" r="3.2" fill="#ffffff" filter="url(#softGlow)" />
        <circle cx="850" cy="360" r="2" fill="#38bdf8" />
        <circle cx="1120" cy="430" r="3" fill="#00ffff" />
    </svg>
</div>

<div id="bottom-taskbar">
    <div class="taskbar-left">
        <button class="btn-switch-desktop" onclick="handleSwitchDesktop()">
            <span>🖥️</span>
            <span>Switch to Desktop</span>
        </button>
    </div>

    <div class="taskbar-center">
        <div class="task-item" onclick="handleFocusBrowser()">
            <span class="shield-icon">🛡️</span>
            <span>Bitdefender SAFEPAY™</span>
        </div>
    </div>

    <div class="taskbar-right">
        <span>Default printer: <strong>Microsoft Print to PDF</strong></span>
        <span class="separator">|</span>
        <span class="status-badge">
            <span class="shield-icon">🛡️</span>
            <span>Bitdefender VPN</span>
        </span>
        <span class="separator">|</span>
        <span>EN</span>
        <span class="separator">|</span>
        <span title="Battery status">🔋</span>
        <span title="Audio status">🔊</span>
        <span class="separator">|</span>
        <span id="live-clock">--/--/---- --:--</span>
    </div>
</div>

<script>
    function updateClock() {
        const now = new Date();
        const pad = (n) => String(n).padStart(2, '0');
        const timeStr = pad(now.getHours()) + ':' + pad(now.getMinutes());
        const dateStr = pad(now.getDate()) + '/' + pad(now.getMonth() + 1) + '/' + now.getFullYear();
        document.getElementById('live-clock').textContent = dateStr + ' ' + timeStr;
    }
    updateClock();
    setInterval(updateClock, 1000);

    function postIpc(msg) {
        if (window.ipc && window.ipc.postMessage) {
            window.ipc.postMessage(JSON.stringify(msg));
        }
    }

    function handleSwitchDesktop() {
        postIpc({ type: 'SWITCH_DESKTOP' });
    }

    function handleFocusBrowser() {
        postIpc({ type: 'FOCUS_BROWSER' });
    }
</script>
</body>
</html>"##
    .to_string()
}

/// Generates the HTML for the top chrome of the floating browser window.
/// Matches Bitdefender SafePay browser window chrome (screenshot 2):
/// - Dark titlebar with shield icon, title, and minimize/maximize/close buttons.
/// - Tab strip with active tab in white rounded style, inactive tabs in dark style, "+" button, Bookmarks & Settings.
/// - Navigation bar with Back, Forward, Reload, URL Omnibox, Virtual Keyboard toggle, and Bookmark action.
pub fn generate_browser_chrome_html(tabs: &[TabItem], active_id: usize) -> String {
    let tabs_json = serde_json::to_string(tabs).unwrap_or_else(|_| "[]".to_string());
    let active_tab = tabs.iter().find(|t| t.id == active_id).or_else(|| tabs.first());
    let initial_url = active_tab.map(|t| t.url.as_str()).unwrap_or("https://duckduckgo.com");

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SafeBrowse Chrome</title>
<style>
    :root {{
        --titlebar-bg: #16181f;
        --tabstrip-bg: #16181f;
        --tab-inactive-bg: #232733;
        --tab-inactive-text: #94a3b8;
        --tab-active-bg: #ffffff;
        --tab-active-text: #0f172a;
        --navbar-bg: #f3f4f6;
        --border-color: #cbd5e1;
        --btn-hover: #e2e8f0;
        --accent-blue: #0284c7;
        --accent-red: #ef4444;
    }}
    * {{
        margin: 0;
        padding: 0;
        box-sizing: border-box;
        user-select: none;
    }}
    html, body {{
        width: 100%;
        height: 100%;
        overflow: hidden;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        background: var(--titlebar-bg);
        display: flex;
        flex-direction: column;
    }}

    /* Row 1: Title Bar */
    #titlebar {{
        height: 32px;
        background: var(--titlebar-bg);
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 4px 0 12px;
        color: #f1f5f9;
        font-size: 12px;
        font-weight: 600;
        cursor: grab;
    }}
    .title-left {{
        display: flex;
        align-items: center;
        gap: 8px;
        pointer-events: none;
    }}
    .title-shield {{
        color: #ef4444;
        font-size: 14px;
    }}
    .window-controls {{
        display: flex;
        align-items: center;
        cursor: default;
    }}
    .win-btn {{
        width: 44px;
        height: 32px;
        background: transparent;
        border: none;
        color: #cbd5e1;
        font-size: 13px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: background 0.1s;
    }}
    .win-btn:hover {{
        background: rgba(255, 255, 255, 0.1);
    }}
    .win-btn.close:hover {{
        background: #e81123;
        color: #fff;
    }}

    /* Row 2: Tab Strip */
    #tabstrip {{
        height: 38px;
        background: var(--tabstrip-bg);
        display: flex;
        align-items: flex-end;
        padding: 0 8px;
        gap: 4px;
        overflow-x: auto;
    }}
    #tabstrip::-webkit-scrollbar {{ display: none; }}
    .tab {{
        height: 34px;
        min-width: 110px;
        max-width: 200px;
        padding: 0 10px;
        border-radius: 6px 6px 0 0;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
        font-size: 12px;
        cursor: pointer;
        transition: all 0.15s;
    }}
    .tab.inactive {{
        background: var(--tab-inactive-bg);
        color: var(--tab-inactive-text);
        border: 1px solid rgba(255,255,255,0.06);
        border-bottom: none;
    }}
    .tab.inactive:hover {{
        background: #2c3244;
        color: #f1f5f9;
    }}
    .tab.active {{
        background: var(--tab-active-bg);
        color: var(--tab-active-text);
        font-weight: 600;
        box-shadow: 0 -2px 6px rgba(0,0,0,0.12);
    }}
    .tab-title {{
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        flex: 1;
    }}
    .tab-close {{
        width: 16px;
        height: 16px;
        border-radius: 50%;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 11px;
        color: inherit;
        opacity: 0.6;
        cursor: pointer;
    }}
    .tab-close:hover {{
        opacity: 1;
        background: rgba(0, 0, 0, 0.15);
    }}
    .btn-new-tab {{
        width: 28px;
        height: 28px;
        border-radius: 4px;
        border: none;
        background: transparent;
        color: #94a3b8;
        font-size: 18px;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        margin-bottom: 3px;
    }}
    .btn-new-tab:hover {{
        background: rgba(255,255,255,0.12);
        color: #fff;
    }}

    /* Row 3: Navigation Bar */
    #navbar {{
        height: 40px;
        background: var(--navbar-bg);
        border-top: 1px solid #e2e8f0;
        border-bottom: 1px solid #cbd5e1;
        display: flex;
        align-items: center;
        padding: 0 10px;
        gap: 6px;
    }}
    .nav-btn {{
        width: 28px;
        height: 28px;
        border-radius: 4px;
        border: 1px solid transparent;
        background: transparent;
        color: #334155;
        font-size: 14px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: all 0.15s;
    }}
    .nav-btn:hover {{
        background: var(--btn-hover);
        border-color: #cbd5e1;
    }}
    .omnibox-wrapper {{
        flex: 1;
        display: flex;
        align-items: center;
        background: #ffffff;
        border: 1px solid #cbd5e1;
        border-radius: 18px;
        padding: 0 12px;
        height: 28px;
        transition: border-color 0.2s, box-shadow 0.2s;
    }}
    .omnibox-wrapper:focus-within {{
        border-color: var(--accent-blue);
        box-shadow: 0 0 0 2px rgba(2, 132, 199, 0.15);
    }}
    .lock-icon {{
        font-size: 12px;
        color: #10b981;
        margin-right: 6px;
    }}
    #omnibox {{
        flex: 1;
        border: none;
        background: transparent;
        outline: none;
        font-size: 12px;
        color: #0f172a;
    }}
    .osk-circle-btn {{
        width: 20px;
        height: 20px;
        border-radius: 50%;
        background: #0284c7;
        color: #fff;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 11px;
        cursor: pointer;
        margin-left: 6px;
        transition: opacity 0.15s;
    }}
    .osk-circle-btn:hover {{
        opacity: 0.85;
    }}
    .action-btn {{
        background: #f1f5f9;
        border: 1px solid #cbd5e1;
        border-radius: 4px;
        height: 28px;
        padding: 0 8px;
        display: flex;
        align-items: center;
        gap: 4px;
        font-size: 11px;
        font-weight: 600;
        color: #334155;
        cursor: pointer;
        transition: all 0.15s;
    }}
    .action-btn:hover {{
        background: #e2e8f0;
        color: #0f172a;
    }}
    .action-btn.active {{
        background: #0284c7;
        color: #ffffff;
        border-color: #0284c7;
    }}

    /* Virtual Keyboard Drawer */
    #osk-drawer {{
        position: fixed;
        bottom: 0;
        left: 50%;
        transform: translateX(-50%);
        width: 760px;
        background: #181b26;
        border: 2px solid #2e3646;
        border-bottom: none;
        border-radius: 10px 10px 0 0;
        box-shadow: 0 -8px 24px rgba(0,0,0,0.6);
        padding: 12px;
        z-index: 9999;
        display: none;
        flex-direction: column;
        gap: 6px;
    }}
    .osk-header {{
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding-bottom: 6px;
        border-bottom: 1px solid rgba(255,255,255,0.08);
        color: #38bdf8;
        font-size: 11px;
        font-weight: 700;
    }}
    .osk-row {{
        display: flex;
        gap: 5px;
        justify-content: center;
    }}
    .osk-key {{
        background: #262c3d;
        color: #fff;
        border: 1px solid #3b455c;
        border-radius: 4px;
        min-width: 40px;
        height: 36px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.1s;
    }}
    .osk-key:hover {{
        background: #39425a;
        border-color: #536282;
    }}
    .osk-key:active {{
        background: #38bdf8;
        color: #000;
        transform: scale(0.96);
    }}
    .osk-key.wide {{ min-width: 64px; }}
    .osk-key.space {{ flex: 1; }}
</style>
</head>
<body>

<div id="titlebar" onmousedown="handleTitlebarMouseDown(event)">
    <div class="title-left">
        <span class="title-shield">🛡️</span>
        <span>Bitdefender SAFEPAY™</span>
    </div>
    <div class="window-controls">
        <button class="win-btn" onclick="handleMinimize()" title="Minimize">─</button>
        <button class="win-btn" onclick="handleMaximize()" title="Maximize">□</button>
        <button class="win-btn close" onclick="handleClose()" title="Close">✕</button>
    </div>
</div>

<div id="tabstrip">
    <div id="tabs-container" style="display: flex; gap: 4px;"></div>
    <button class="btn-new-tab" onclick="handleNewTab()" title="New Tab">+</button>
</div>

<div id="navbar">
    <button class="nav-btn" onclick="handleBack()" title="Back">⮜</button>
    <button class="nav-btn" onclick="handleForward()" title="Forward">⮞</button>
    <button class="nav-btn" onclick="handleReload()" title="Reload">⟳</button>
    
    <div class="omnibox-wrapper">
        <span class="lock-icon" id="lock-indicator">🔒</span>
        <input type="text" id="omnibox" value="{initial_url}" placeholder="Search securely or type a web address...">
        <div class="osk-circle-btn" onclick="toggleOsk()" title="Virtual Keyboard">⌨️</div>
    </div>

    <button class="action-btn" onclick="handleAddBookmark()" title="Bookmark this page">
        <span>⭐</span>
    </button>
    <button class="action-btn" onclick="handleOpenSettings()" title="Settings">
        <span>⚙️</span>
    </button>
</div>

<!-- Secure Virtual Keyboard Drawer -->
<div id="osk-drawer">
    <div class="osk-header">
        <span>🛡️ SECURE VIRTUAL KEYBOARD (Direct DOM Injection)</span>
        <div style="display: flex; gap: 6px;">
            <button style="padding: 2px 8px; font-size: 10px; cursor: pointer; background: #262c3d; color: #fff; border: 1px solid #475569; border-radius: 3px;" onclick="scrambleKeys()">🎲 Scramble Keys</button>
            <button style="padding: 2px 8px; font-size: 10px; cursor: pointer; background: #262c3d; color: #fff; border: 1px solid #475569; border-radius: 3px;" onclick="toggleOsk()">✕ Close</button>
        </div>
    </div>
    <div id="osk-keys-container"></div>
</div>

<script>
    let currentTabs = {tabs_json};
    let activeTabId = {active_id};
    const omnibox = document.getElementById('omnibox');
    const tabsContainer = document.getElementById('tabs-container');
    const oskDrawer = document.getElementById('osk-drawer');
    let isShifted = false;

    function postIpc(msg) {{
        if (window.ipc && window.ipc.postMessage) {{
            window.ipc.postMessage(JSON.stringify(msg));
        }}
    }}

    function handleTitlebarMouseDown(e) {{
        if (e.target.closest('.window-controls')) return;
        postIpc({{ type: 'START_DRAG' }});
    }}

    function handleMinimize() {{ postIpc({{ type: 'MINIMIZE' }}); }}
    function handleMaximize() {{ postIpc({{ type: 'TOGGLE_MAXIMIZE' }}); }}
    function handleClose() {{ postIpc({{ type: 'CLOSE_WINDOW' }}); }}

    function handleBack() {{ postIpc({{ type: 'GO_BACK' }}); }}
    function handleForward() {{ postIpc({{ type: 'GO_FORWARD' }}); }}
    function handleReload() {{ postIpc({{ type: 'RELOAD' }}); }}
    function handleNewTab() {{ postIpc({{ type: 'NEW_TAB' }}); }}
    function handleOpenBookmarks() {{ postIpc({{ type: 'OPEN_BOOKMARKS' }}); }}
    function handleOpenSettings() {{ postIpc({{ type: 'OPEN_SETTINGS' }}); }}
    function handleAddBookmark() {{ postIpc({{ type: 'ADD_BOOKMARK' }}); }}

    omnibox.addEventListener('keydown', (e) => {{
        if (e.key === 'Enter') {{
            let target = omnibox.value.trim();
            if (!target.startsWith('http://') && !target.startsWith('https://')) {{
                if (target.includes('.') && !target.includes(' ')) {{
                    target = 'https://' + target;
                }} else {{
                    target = 'https://duckduckgo.com/?q=' + encodeURIComponent(target);
                }}
            }}
            omnibox.value = target;
            postIpc({{ type: 'NAVIGATE', url: target }});
        }}
    }});

    function renderTabs() {{
        tabsContainer.innerHTML = '';

        currentTabs.forEach(t => {{
            const isTabActive = t.id === activeTabId;
            const tabDiv = document.createElement('div');
            tabDiv.className = 'tab ' + (isTabActive ? 'active' : 'inactive');

            let icon = '🌐';
            if (t.kind === 'Bookmarks') {{ icon = '⭐'; }}
            else if (t.kind === 'Settings') {{ icon = '⚙️'; }}

            tabDiv.innerHTML = `
                <span>${{icon}}</span>
                <span class="tab-title">${{t.title || 'New Tab'}}</span>
                <span class="tab-close" onclick="closeTab(event, ${{t.id}})">✕</span>
            `;

            tabDiv.onclick = (e) => {{
                if (e.target.classList.contains('tab-close')) return;
                postIpc({{ type: 'SWITCH_TAB', id: t.id }});
            }};

            tabsContainer.appendChild(tabDiv);
        }});

        // Dedicated Bookmarks and Settings tabs in tabstrip if not present
        const hasBookmarks = currentTabs.some(t => t.kind === 'Bookmarks');
        if (!hasBookmarks) {{
            const bmDiv = document.createElement('div');
            bmDiv.className = 'tab inactive';
            bmDiv.innerHTML = `<span>⭐</span><span class="tab-title">Bookmarks</span>`;
            bmDiv.onclick = handleOpenBookmarks;
            tabsContainer.appendChild(bmDiv);
        }}

        const hasSettings = currentTabs.some(t => t.kind === 'Settings');
        if (!hasSettings) {{
            const stDiv = document.createElement('div');
            stDiv.className = 'tab inactive';
            stDiv.innerHTML = `<span>⚙️</span><span class="tab-title">Settings</span>`;
            stDiv.onclick = handleOpenSettings;
            tabsContainer.appendChild(stDiv);
        }}

        const activeObj = currentTabs.find(t => t.id === activeTabId);
        if (activeObj) {{
            omnibox.value = activeObj.url;
            document.getElementById('lock-indicator').style.color = activeObj.is_secure ? '#10b981' : '#94a3b8';
        }}
    }}

    function closeTab(e, id) {{
        e.stopPropagation();
        postIpc({{ type: 'CLOSE_TAB', id: id }});
    }}

    window.updateTabs = function(tabs, activeId) {{
        currentTabs = tabs;
        activeTabId = activeId;
        renderTabs();
    }};

    renderTabs();

    // Virtual Keyboard
    const defaultLayout = [
        ['1','2','3','4','5','6','7','8','9','0','-','=','BACKSPACE'],
        ['q','w','e','r','t','y','u','i','o','p','[',']','\\'],
        ['CAPS','a','s','d','f','g','h','j','k','l',';','\'','ENTER'],
        ['SHIFT','z','x','c','v','b','n','m',',','.','/','SHIFT'],
        ['SPACE']
    ];
    let currentLayout = JSON.parse(JSON.stringify(defaultLayout));

    function toggleOsk() {{
        const isVisible = oskDrawer.style.display === 'flex';
        oskDrawer.style.display = isVisible ? 'none' : 'flex';
        if (!isVisible) renderOsk();
    }}

    function renderOsk() {{
        const container = document.getElementById('osk-keys-container');
        container.innerHTML = '';
        currentLayout.forEach(row => {{
            const rowDiv = document.createElement('div');
            rowDiv.className = 'osk-row';
            row.forEach(key => {{
                const btn = document.createElement('div');
                btn.className = 'osk-key';
                let display = key;
                if (isShifted && display.length === 1 && display >= 'a' && display <= 'z') {{
                    display = display.toUpperCase();
                }}
                btn.textContent = display;
                if (key === 'BACKSPACE' || key === 'ENTER' || key === 'CAPS' || key === 'SHIFT') btn.classList.add('wide');
                else if (key === 'SPACE') {{ btn.classList.add('space'); btn.textContent = 'SPACE'; }}
                btn.onclick = () => handleKeyClick(key);
                rowDiv.appendChild(btn);
            }});
            container.appendChild(rowDiv);
        }});
    }}

    function handleKeyClick(key) {{
        if (key === 'SHIFT' || key === 'CAPS') {{
            isShifted = !isShifted;
            renderOsk();
            return;
        }}
        let val = key;
        if (key === 'SPACE') val = ' ';
        else if (isShifted && key.length === 1 && key >= 'a' && key <= 'z') val = key.toUpperCase();

        if (document.activeElement === omnibox) {{
            if (val === 'BACKSPACE') {{
                omnibox.value = omnibox.value.slice(0, -1);
            }} else if (val === 'ENTER') {{
                omnibox.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter' }}));
            }} else {{
                omnibox.value += val;
            }}
            return;
        }}

        postIpc({{ type: 'KEY_INPUT', action: val }});
    }}

    function scrambleKeys() {{
        const alphas = [];
        defaultLayout.forEach(row => row.forEach(k => {{
            if (k.length === 1 && k >= 'a' && k <= 'z') alphas.push(k);
        }}));
        for (let i = alphas.length - 1; i > 0; i--) {{
            const j = Math.floor(Math.random() * (i + 1));
            [alphas[i], alphas[j]] = [alphas[j], alphas[i]];
        }}
        let idx = 0;
        currentLayout = defaultLayout.map(row => row.map(k => (k.length === 1 && k >= 'a' && k <= 'z') ? alphas[idx++] : k));
        renderOsk();
    }}
</script>
</body>
</html>"##,
        initial_url = initial_url,
        tabs_json = tabs_json,
        active_id = active_id
    )
}

/// Generates the HTML for the Bookmarks screen matching SafePay screenshot 3.
/// Displays large "Bookmarks" header, "+ " tile, and responsive tiles for bookmarked sites.
pub fn generate_bookmarks_page_html(bookmarks: &[Bookmark]) -> String {
    let mut tiles_html = String::new();

    for b in bookmarks {
        let domain = url::Url::parse(&b.url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| b.url.clone());

        tiles_html.push_str(&format!(
            r##"<div class="bm-card" onclick="openBookmark('{url}')">
                <div class="bm-icon-circle">🌐</div>
                <div class="bm-card-title">{title}</div>
                <div class="bm-card-domain">{domain}</div>
            </div>"##,
            url = b.url,
            title = b.title,
            domain = domain
        ));
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Bookmarks</title>
<style>
    * {{
        margin: 0;
        padding: 0;
        box-sizing: border-box;
        user-select: none;
    }}
    body {{
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        background: #f8fafc;
        color: #1e293b;
        padding: 48px 64px;
    }}
    h1 {{
        font-size: 32px;
        font-weight: 700;
        color: #0f172a;
        margin-bottom: 8px;
    }}
    .subtitle {{
        font-size: 15px;
        color: #64748b;
        margin-bottom: 40px;
    }}
    .grid {{
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
        gap: 24px;
    }}
    .add-tile {{
        height: 140px;
        background: #e2e8f0;
        border: 2px dashed #94a3b8;
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        transition: all 0.2s;
        font-size: 36px;
        color: #475569;
    }}
    .add-tile:hover {{
        background: #cbd5e1;
        border-color: #0284c7;
        color: #0284c7;
        transform: translateY(-2px);
    }}
    .add-label {{
        font-size: 12px;
        font-weight: 600;
        margin-top: 6px;
    }}
    .bm-card {{
        height: 140px;
        background: #ffffff;
        border: 1px solid #e2e8f0;
        border-radius: 8px;
        padding: 16px;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        text-align: center;
        cursor: pointer;
        box-shadow: 0 2px 4px rgba(0,0,0,0.04);
        transition: all 0.2s;
    }}
    .bm-card:hover {{
        border-color: #0284c7;
        box-shadow: 0 6px 16px rgba(2, 132, 199, 0.12);
        transform: translateY(-3px);
    }}
    .bm-icon-circle {{
        width: 44px;
        height: 44px;
        border-radius: 50%;
        background: #f1f5f9;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 20px;
        margin-bottom: 12px;
    }}
    .bm-card-title {{
        font-size: 13px;
        font-weight: 600;
        color: #0f172a;
        max-width: 100%;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }}
    .bm-card-domain {{
        font-size: 11px;
        color: #64748b;
        margin-top: 4px;
    }}
</style>
</head>
<body>

<h1>Bookmarks</h1>
<div class="subtitle">Bookmark your favorite webpages for quick access.</div>

<div class="grid">
    <div class="add-tile" onclick="promptAddBookmark()">
        <span>+</span>
        <span class="add-label">Add Bookmark</span>
    </div>
    {tiles_html}
</div>

<script>
    function postIpc(msg) {{
        if (window.ipc && window.ipc.postMessage) {{
            window.ipc.postMessage(JSON.stringify(msg));
        }}
    }}

    function openBookmark(url) {{
        postIpc({{ type: 'NAVIGATE', url: url }});
    }}

    function promptAddBookmark() {{
        const title = prompt('Enter Bookmark Name:');
        if (!title) return;
        let url = prompt('Enter Bookmark URL:', 'https://');
        if (!url) return;
        postIpc({{ type: 'ADD_BOOKMARK_DIRECT', title: title, url: url }});
    }}
</script>
</body>
</html>"##,
        tiles_html = tiles_html
    )
}

/// Generates the HTML for the Settings screen matching SafePay screenshot 4.
/// Includes domain rules, pop-up blocker, virtual keyboard auto-launch, print confirmation, and PDF print toggles.
pub fn generate_settings_page_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Settings</title>
<style>
    * {
        margin: 0;
        padding: 0;
        box-sizing: border-box;
        user-select: none;
    }
    body {
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        background: #f8fafc;
        color: #1e293b;
        padding: 48px 64px;
    }
    h1 {
        font-size: 32px;
        font-weight: 700;
        color: #0f172a;
        margin-bottom: 32px;
    }
    .setting-section {
        background: #ffffff;
        border: 1px solid #e2e8f0;
        border-radius: 8px;
        padding: 20px 24px;
        margin-bottom: 20px;
        box-shadow: 0 1px 3px rgba(0,0,0,0.03);
    }
    .setting-row {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
    }
    .setting-info {
        max-width: 80%;
    }
    .setting-title {
        font-size: 15px;
        font-weight: 600;
        color: #0f172a;
        margin-bottom: 4px;
    }
    .setting-desc {
        font-size: 13px;
        color: #64748b;
        line-height: 1.4;
    }
    .switch {
        position: relative;
        display: inline-block;
        width: 44px;
        height: 24px;
    }
    .switch input { opacity: 0; width: 0; height: 0; }
    .slider {
        position: absolute;
        cursor: pointer;
        top: 0; left: 0; right: 0; bottom: 0;
        background-color: #cbd5e1;
        transition: .3s;
        border-radius: 24px;
    }
    .slider:before {
        position: absolute;
        content: "";
        height: 18px;
        width: 18px;
        left: 3px;
        bottom: 3px;
        background-color: white;
        transition: .3s;
        border-radius: 50%;
    }
    input:checked + .slider { background-color: #0284c7; }
    input:checked + .slider:before { transform: translateX(20px); }

    .rule-box {
        margin-top: 14px;
        background: #f1f5f9;
        border: 1px solid #e2e8f0;
        border-radius: 6px;
        padding: 10px 16px;
        display: flex;
        align-items: center;
        justify-content: space-between;
        font-size: 13px;
    }
    .input-row {
        display: flex;
        gap: 8px;
        margin-top: 12px;
    }
    .input-field {
        flex: 1;
        height: 32px;
        border: 1px solid #cbd5e1;
        border-radius: 4px;
        padding: 0 10px;
        font-size: 12px;
        outline: none;
    }
    .btn-action {
        background: #0284c7;
        color: #fff;
        border: none;
        border-radius: 4px;
        padding: 0 14px;
        font-size: 12px;
        font-weight: 600;
        cursor: pointer;
    }
    .trash-btn {
        background: transparent;
        border: none;
        cursor: pointer;
        color: #94a3b8;
        font-size: 14px;
    }
    .trash-btn:hover { color: #ef4444; }
</style>
</head>
<body>

<h1>Settings</h1>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Apply Bitdefender Safepay™ rules for accessed domains</div>
            <div class="setting-desc">View Bitdefender Safepay domain rules in the list below</div>
        </div>
        <label class="switch">
            <input type="checkbox" checked>
            <span class="slider"></span>
        </label>
    </div>
    <div class="rule-box" id="rule-ubs">
        <div style="display: flex; align-items: center; gap: 8px;">
            <span>🌐</span>
            <strong>ubs.com</strong>
        </div>
        <div style="display: flex; align-items: center; gap: 12px;">
            <span style="color: #64748b;">Do not recommend me to use Safepay™</span>
            <button class="trash-btn" onclick="document.getElementById('rule-ubs').remove()">🗑️</button>
        </div>
    </div>
</div>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Block pop-ups</div>
            <div class="setting-desc">Blocking pop-ups will reduce the chance of compromising your device.</div>
        </div>
        <label class="switch">
            <input type="checkbox" checked>
            <span class="slider"></span>
        </label>
    </div>
    <div class="input-row">
        <input type="text" id="popup-input" class="input-field" placeholder="Allow pop-ups from these domains:">
        <button class="btn-action" onclick="addPopupDomain()">Add domain</button>
    </div>
    <div id="popup-domains-list">
        <div class="rule-box" id="rule-temu">
            <div style="display: flex; align-items: center; gap: 8px;">
                <span>🌐</span>
                <strong>temu.com</strong>
            </div>
            <button class="trash-btn" onclick="document.getElementById('rule-temu').remove()">🗑️</button>
        </div>
    </div>
</div>

<div style="margin-bottom: 24px; font-size: 12px; color: #94a3b8; font-style: italic;">
    Adobe Flash Player support has been removed due to its EOL announcement.
</div>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Manage certificates</div>
            <div class="setting-desc">Import existing certificates from file.</div>
        </div>
        <button class="btn-action" style="height: 28px;" onclick="alert('Certificate manager ready.')">Import</button>
    </div>
</div>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Use Virtual Keyboard</div>
            <div class="setting-desc">Automatically launch Virtual Keyboard when password fields are selected.</div>
        </div>
        <label class="switch">
            <input type="checkbox" checked>
            <span class="slider"></span>
        </label>
    </div>
</div>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Printing confirmation</div>
            <div class="setting-desc">Ask for confirmation before printing pages.</div>
        </div>
        <label class="switch">
            <input type="checkbox">
            <span class="slider"></span>
        </label>
    </div>
</div>

<div class="setting-section">
    <div class="setting-row">
        <div class="setting-info">
            <div class="setting-title">Allow print to PDF</div>
            <div class="setting-desc">Enables you to save files in a printable PDF format using Microsoft Print to PDF.</div>
        </div>
        <label class="switch">
            <input type="checkbox">
            <span class="slider"></span>
        </label>
    </div>
</div>

<script>
    function addPopupDomain() {
        const inp = document.getElementById('popup-input');
        const val = inp.value.trim();
        if (!val) return;
        const list = document.getElementById('popup-domains-list');
        const box = document.createElement('div');
        box.className = 'rule-box';
        box.innerHTML = `
            <div style="display: flex; align-items: center; gap: 8px;">
                <span>🌐</span>
                <strong>${val}</strong>
            </div>
            <button class="trash-btn" onclick="this.closest('.rule-box').remove()">🗑️</button>
        `;
        list.appendChild(box);
        inp.value = '';
    }
</script>
</body>
</html>"##
    .to_string()
}

/// Generates the HTML for the Default Desktop Companion Dock Window.
/// Matches screenshot 1: SafePay visible in the Windows 11 taskbar dock.
/// Clicking it or activating it immediately switches display back to SafeBrowseDesktop!
pub fn generate_dock_companion_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SafeBrowse Dock</title>
<style>
    * {
        margin: 0;
        padding: 0;
        box-sizing: border-box;
        user-select: none;
    }
    body {
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        background: #0f1422;
        color: #f1f5f9;
        display: flex;
        flex-direction: column;
        justify-content: center;
        align-items: center;
        height: 100vh;
        padding: 24px;
        text-align: center;
    }
    .shield-icon {
        font-size: 38px;
        color: #ef4444;
        margin-bottom: 10px;
    }
    h2 {
        font-size: 16px;
        font-weight: 700;
        color: #ffffff;
        margin-bottom: 6px;
    }
    p {
        font-size: 12px;
        color: #94a3b8;
        margin-bottom: 20px;
        line-height: 1.4;
    }
    .btn-return {
        background: #0284c7;
        color: #fff;
        border: none;
        border-radius: 6px;
        padding: 10px 20px;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        display: flex;
        align-items: center;
        gap: 8px;
        transition: background 0.15s, transform 0.1s;
        box-shadow: 0 4px 12px rgba(2, 132, 199, 0.4);
    }
    .btn-return:hover {
        background: #0369a1;
        transform: translateY(-1px);
    }
    .btn-return:active {
        transform: translateY(0);
    }
    .btn-exit {
        margin-top: 10px;
        background: transparent;
        color: #94a3b8;
        border: 1px solid #334155;
        border-radius: 4px;
        padding: 6px 14px;
        font-size: 11px;
        cursor: pointer;
    }
    .btn-exit:hover {
        color: #ef4444;
        border-color: #ef4444;
    }
</style>
</head>
<body>

<div class="shield-icon">🛡️</div>
<h2>Bitdefender SAFEPAY™ Active</h2>
<p>SafeBrowse is running inside an isolated secure desktop.<br>Click below or press <strong>Ctrl+Alt+D</strong> to return.</p>

<button class="btn-return" onclick="handleReturn()">
    <span>🖥️</span>
    <span>Return to SafeBrowse</span>
</button>

<button class="btn-exit" onclick="handleExit()">
    <span>✕ Terminate Session</span>
</button>

<script>
    function postIpc(msg) {
        if (window.ipc && window.ipc.postMessage) {
            window.ipc.postMessage(JSON.stringify(msg));
        }
    }
    function handleReturn() {
        postIpc({ type: 'SWITCH_TO_SAFE_DESKTOP' });
    }
    function handleExit() {
        postIpc({ type: 'TERMINATE_SESSION' });
    }
</script>
</body>
</html>"##
    .to_string()
}
