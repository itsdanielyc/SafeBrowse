//! Embedded Assets & UI Shell for SafeBrowse
//!
//! Provides the hardened top chrome bar, omnibox, tab manager,
//! quick bookmarks bar, and floating secure virtual keyboard.

/// Generates the HTML, CSS, and JS injection script that wraps or overlays the browser view.
pub fn generate_kiosk_shell_html(initial_url: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>SafeBrowse - Secure Isolated Desktop</title>
<style>
    :root {{
        --bg-dark: #12141a;
        --bg-surface: #1a1d26;
        --bg-hover: #262b38;
        --accent-green: #00e676;
        --accent-blue: #2979ff;
        --accent-red: #ff1744;
        --text-primary: #f0f4f8;
        --text-secondary: #94a3b8;
        --border-color: #2e3646;
    }}
    * {{
        box-sizing: border-box;
        margin: 0;
        padding: 0;
        user-select: none;
    }}
    body, html {{
        width: 100%;
        height: 100%;
        overflow: hidden;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        background: var(--bg-dark);
        color: var(--text-primary);
        display: flex;
        flex-direction: column;
    }}
    #topbar {{
        background: var(--bg-surface);
        border-bottom: 2px solid var(--border-color);
        display: flex;
        flex-direction: column;
        z-index: 1000;
        box-shadow: 0 4px 12px rgba(0,0,0,0.5);
    }}
    .status-row {{
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 6px 16px;
        background: #0f1117;
        font-size: 12px;
        border-bottom: 1px solid var(--border-color);
    }}
    .security-badge {{
        display: flex;
        align-items: center;
        gap: 8px;
        color: var(--accent-green);
        font-weight: 700;
        letter-spacing: 0.5px;
    }}
    .pulse-dot {{
        width: 8px;
        height: 8px;
        background: var(--accent-green);
        border-radius: 50%;
        box-shadow: 0 0 8px var(--accent-green);
        animation: pulse 2s infinite;
    }}
    @keyframes pulse {{
        0% {{ opacity: 0.4; }}
        50% {{ opacity: 1; }}
        100% {{ opacity: 0.4; }}
    }}
    .desktop-actions {{
        display: flex;
        align-items: center;
        gap: 10px;
    }}
    .btn-switch-desktop {{
        background: #1e293b;
        color: #38bdf8;
        border: 1px solid #38bdf8;
        padding: 4px 10px;
        border-radius: 4px;
        cursor: pointer;
        font-weight: 600;
        font-size: 11px;
        transition: all 0.2s;
    }}
    .btn-switch-desktop:hover {{
        background: #38bdf8;
        color: #0f172a;
    }}
    .btn-exit {{
        background: #3f1d24;
        color: var(--accent-red);
        border: 1px solid var(--accent-red);
        padding: 4px 10px;
        border-radius: 4px;
        cursor: pointer;
        font-weight: 600;
        font-size: 11px;
        transition: all 0.2s;
    }}
    .btn-exit:hover {{
        background: var(--accent-red);
        color: #fff;
    }}
    .nav-row {{
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 8px 16px;
    }}
    .nav-btn {{
        background: var(--bg-hover);
        color: var(--text-primary);
        border: 1px solid var(--border-color);
        width: 34px;
        height: 34px;
        border-radius: 6px;
        display: flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        font-size: 15px;
        transition: background 0.15s;
    }}
    .nav-btn:hover {{
        background: #333a4d;
    }}
    .nav-btn:active {{
        transform: scale(0.95);
    }}
    .omnibox-container {{
        flex: 1;
        display: flex;
        align-items: center;
        background: #0b0d13;
        border: 1px solid var(--border-color);
        border-radius: 6px;
        padding: 0 12px;
        height: 36px;
        transition: border-color 0.2s;
    }}
    .omnibox-container:focus-within {{
        border-color: var(--accent-green);
        box-shadow: 0 0 6px rgba(0,230,118,0.2);
    }}
    .lock-icon {{
        color: var(--accent-green);
        margin-right: 8px;
        font-size: 14px;
    }}
    #omnibox {{
        flex: 1;
        background: transparent;
        border: none;
        outline: none;
        color: var(--text-primary);
        font-size: 13px;
    }}
    .tool-btn {{
        background: var(--bg-hover);
        color: var(--text-primary);
        border: 1px solid var(--border-color);
        padding: 0 12px;
        height: 34px;
        border-radius: 6px;
        cursor: pointer;
        font-size: 12px;
        font-weight: 600;
        display: flex;
        align-items: center;
        gap: 6px;
        transition: all 0.2s;
    }}
    .tool-btn:hover {{
        background: #333a4d;
    }}
    .tool-btn.active {{
        background: rgba(0, 230, 118, 0.15);
        border-color: var(--accent-green);
        color: var(--accent-green);
    }}
    #bookmark-bar {{
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 4px 16px;
        background: #151821;
        border-top: 1px solid rgba(255,255,255,0.05);
        overflow-x: auto;
        font-size: 12px;
    }}
    .bm-chip {{
        background: var(--bg-surface);
        padding: 4px 10px;
        border-radius: 4px;
        border: 1px solid var(--border-color);
        cursor: pointer;
        white-space: nowrap;
        display: flex;
        align-items: center;
        gap: 6px;
        color: var(--text-secondary);
        transition: all 0.15s;
    }}
    .bm-chip:hover {{
        color: var(--text-primary);
        border-color: #4a5568;
        background: var(--bg-hover);
    }}
    #content-frame {{
        flex: 1;
        width: 100%;
        border: none;
        background: #fff;
    }}
    #osk-drawer {{
        position: fixed;
        bottom: 0;
        left: 50%;
        transform: translateX(-50%);
        width: 780px;
        background: #1a1d26;
        border: 2px solid var(--border-color);
        border-bottom: none;
        border-radius: 12px 12px 0 0;
        box-shadow: 0 -8px 24px rgba(0,0,0,0.6);
        padding: 16px;
        z-index: 2000;
        display: none;
        flex-direction: column;
        gap: 8px;
    }}
    .osk-header {{
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 6px;
    }}
    .osk-title {{
        font-size: 12px;
        font-weight: 700;
        color: var(--accent-green);
        display: flex;
        align-items: center;
        gap: 6px;
    }}
    .osk-row {{
        display: flex;
        gap: 6px;
        justify-content: center;
    }}
    .osk-key {{
        background: #282e3f;
        color: #fff;
        border: 1px solid #3c465c;
        border-radius: 6px;
        min-width: 44px;
        height: 42px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 14px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.1s;
    }}
    .osk-key:hover {{
        background: #3a435b;
        border-color: #556280;
    }}
    .osk-key:active {{
        background: var(--accent-green);
        color: #000;
        transform: scale(0.95);
    }}
    .osk-key.wide {{ min-width: 70px; }}
    .osk-key.extra-wide {{ min-width: 90px; }}
    .osk-key.space {{ flex: 1; }}
</style>
</head>
<body>

<div id="topbar">
    <div class="status-row">
        <div class="security-badge">
            <div class="pulse-dot"></div>
            <span>SAFEBROWSE ISOLATED DESKTOP</span>
            <span style="color: #64748b; font-weight: normal;">| DWM Capture Excluded | User Hooks Blocked</span>
        </div>
        <div class="desktop-actions">
            <button class="btn-switch-desktop" onclick="handleSwitchDesktop()">🖥️ Back to Windows Desktop (Ctrl+Alt+D)</button>
            <button class="btn-exit" onclick="handleExit()">❌ Exit SafeBrowse</button>
        </div>
    </div>
    <div class="nav-row">
        <button class="nav-btn" onclick="handleBack()" title="Back">⮜</button>
        <button class="nav-btn" onclick="handleForward()" title="Forward">⮞</button>
        <button class="nav-btn" onclick="handleReload()" title="Reload">⟳</button>
        <button class="nav-btn" onclick="handleHome()" title="Home">🏠</button>
        
        <div class="omnibox-container">
            <span class="lock-icon">🔒</span>
            <input type="text" id="omnibox" value="{initial_url}" placeholder="Search securely or enter web address...">
        </div>
        
        <button class="tool-btn" id="btn-osk" onclick="toggleOsk()">⌨️ Virtual Keyboard</button>
        <button class="tool-btn" onclick="addCurrentBookmark()">⭐ Bookmark</button>
    </div>
    <div id="bookmark-bar">
        <div class="bm-chip" onclick="navigateTo('https://duckduckgo.com')">🦆 DuckDuckGo</div>
        <div class="bm-chip" onclick="navigateTo('https://www.paypal.com')">💳 PayPal</div>
        <div class="bm-chip" onclick="navigateTo('https://dashboard.stripe.com')">⚡ Stripe</div>
        <div class="bm-chip" onclick="navigateTo('https://www.chase.com')">🏦 Chase</div>
        <div class="bm-chip" onclick="navigateTo('https://www.bankofamerica.com')">🏛️ Bank of America</div>
        <div class="bm-chip" onclick="navigateTo('https://www.fidelity.com')">📈 Fidelity</div>
    </div>
</div>

<iframe id="content-frame" src="{initial_url}"></iframe>

<!-- Floating Trusted Virtual Keyboard -->
<div id="osk-drawer">
    <div class="osk-header">
        <div class="osk-title">🛡️ SECURE VIRTUAL KEYBOARD (Hook-Immune DOM Injection)</div>
        <div style="display: flex; gap: 8px;">
            <button class="btn-switch-desktop" onclick="scrambleKeys()">🎲 Scramble Keys</button>
            <button class="btn-exit" onclick="toggleOsk()">✕ Close</button>
        </div>
    </div>
    <div id="osk-keys-container"></div>
</div>

<script>
    const omnibox = document.getElementById('omnibox');
    const iframe = document.getElementById('content-frame');
    const oskDrawer = document.getElementById('osk-drawer');
    const btnOsk = document.getElementById('btn-osk');

    let isShifted = false;

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
            navigateTo(target);
        }}
    }});

    function navigateTo(url) {{
        omnibox.value = url;
        iframe.src = url;
        postIpc({{ type: 'NAVIGATE', url: url }});
    }}

    function handleBack() {{
        try {{ iframe.contentWindow.history.back(); }} catch(e) {{}}
        postIpc({{ type: 'GO_BACK' }});
    }}

    function handleForward() {{
        try {{ iframe.contentWindow.history.forward(); }} catch(e) {{}}
        postIpc({{ type: 'GO_FORWARD' }});
    }}

    function handleReload() {{
        try {{ iframe.contentWindow.location.reload(); }} catch(e) {{}}
        postIpc({{ type: 'RELOAD' }});
    }}

    function handleHome() {{
        navigateTo('https://duckduckgo.com');
    }}

    function handleSwitchDesktop() {{
        postIpc({{ type: 'SWITCH_DESKTOP' }});
    }}

    function handleExit() {{
        postIpc({{ type: 'EXIT_APP' }});
    }}

    function toggleOsk() {{
        const isVisible = oskDrawer.style.display === 'flex';
        oskDrawer.style.display = isVisible ? 'none' : 'flex';
        btnOsk.classList.toggle('active', !isVisible);
        if (!isVisible) {{
            renderOsk();
        }}
    }}

    function addCurrentBookmark() {{
        postIpc({{
            type: 'ADD_BOOKMARK',
            title: document.title || omnibox.value,
            url: omnibox.value
        }});
        alert('Bookmark saved securely to persistent store!');
    }}

    function postIpc(msgObj) {{
        if (window.ipc && window.ipc.postMessage) {{
            window.ipc.postMessage(JSON.stringify(msgObj));
        }}
    }}

    // Virtual Keyboard Layout Definition
    const defaultLayout = [
        ['1','2','3','4','5','6','7','8','9','0','-','=','BACKSPACE'],
        ['q','w','e','r','t','y','u','i','o','p','[',']','\\'],
        ['CAPS','a','s','d','f','g','h','j','k','l',';','\'','ENTER'],
        ['SHIFT','z','x','c','v','b','n','m',',','.','/','SHIFT'],
        ['SPACE']
    ];

    let currentLayout = JSON.parse(JSON.stringify(defaultLayout));

    function renderOsk() {{
        const container = document.getElementById('osk-keys-container');
        container.innerHTML = '';

        currentLayout.forEach((row, rowIdx) => {{
            const rowDiv = document.createElement('div');
            rowDiv.className = 'osk-row';

            row.forEach(keyVal => {{
                const btn = document.createElement('div');
                btn.className = 'osk-key';
                
                let displayVal = keyVal;
                if (isShifted && displayVal.length === 1 && displayVal >= 'a' && displayVal <= 'z') {{
                    displayVal = displayVal.toUpperCase();
                }}

                btn.textContent = displayVal;

                if (keyVal === 'BACKSPACE' || keyVal === 'ENTER' || keyVal === 'CAPS' || keyVal === 'SHIFT') {{
                    btn.classList.add('wide');
                }} else if (keyVal === 'SPACE') {{
                    btn.classList.add('space');
                    btn.textContent = 'SPACE';
                }}

                btn.onclick = () => handleKeyClick(keyVal);
                rowDiv.appendChild(btn);
            }});

            container.appendChild(rowDiv);
        }});
    }}

    function handleKeyClick(keyVal) {{
        if (keyVal === 'SHIFT' || keyVal === 'CAPS') {{
            isShifted = !isShifted;
            renderOsk();
            return;
        }}

        let valueToDispatch = keyVal;
        if (keyVal === 'SPACE') valueToDispatch = ' ';
        else if (isShifted && keyVal.length === 1 && keyVal >= 'a' && keyVal <= 'z') {{
            valueToDispatch = keyVal.toUpperCase();
        }}

        // Send via IPC directly to active document input in host webview
        postIpc({{
            type: 'KEY_INPUT',
            action: valueToDispatch
        }});
    }}

    function scrambleKeys() {{
        // Fisher-Yates shuffle on alpha keys for anti-mouse-logger defense
        const alphas = [];
        defaultLayout.forEach(row => {{
            row.forEach(k => {{
                if (k.length === 1 && k >= 'a' && k <= 'z') alphas.push(k);
            }});
        }});

        for (let i = alphas.length - 1; i > 0; i--) {{
            const j = Math.floor(Math.random() * (i + 1));
            [alphas[i], alphas[j]] = [alphas[j], alphas[i]];
        }}

        let idx = 0;
        currentLayout = defaultLayout.map(row => {{
            return row.map(k => {{
                if (k.length === 1 && k >= 'a' && k <= 'z') {{
                    return alphas[idx++];
                }}
                return k;
            }});
        }});

        renderOsk();
    }}
</script>
</body>
</html>"#,
        initial_url = initial_url
    )
}
