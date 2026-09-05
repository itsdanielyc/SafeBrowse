//! Exports the exact bundled UI into a local browser preview without starting SafeBrowse.
//!
//! Run `cargo run --example ui_preview`, then serve `target/ui-preview` with a
//! local static-file server. Add `-- --capture-permissions` to render sample PNGs
//! in hidden WebView2 fixtures. Neither mode creates a SafeBrowse session or
//! grants real website permissions; all policy data and IPC are preview fixtures.
//! Pass `--capture-permissions` to additionally render sample-only permission
//! surfaces into PNGs using hidden WebView2 fixtures, without starting SafeBrowse.

use safebrowse::bookmarks::{Bookmark, BookmarkCategory};
use safebrowse::browser::tabs::{TabItem, TabKind};
use safebrowse::ui::assets::{
    generate_bookmarks_page_html, generate_browser_chrome_html_with_session,
    generate_capture_warning_html, generate_desktop_shell_html_with_session,
    generate_permission_prompt_html, generate_settings_page_html_with_session,
    generate_virtual_keyboard_html,
};
use std::fs;
use std::io;
use std::path::Path;

const OUTPUT_DIRECTORY: &str = "target/ui-preview";
const PREVIEW_BRIDGE: &str = r#"<script>
window.ipc = { postMessage(message) { parent.postMessage({ safeBrowsePreview: true, command: JSON.parse(message) }, '*'); } };
</script>"#;

/// Inserts the preview-only bridge before any application JavaScript executes.
fn write_page(directory: &Path, filename: &str, html: String) -> io::Result<()> {
    let page = html.replacen("</head>", &format!("{PREVIEW_BRIDGE}</head>"), 1);
    fs::write(directory.join(filename), page)
}

/// Exports deterministic sample data using reserved example.com hostnames.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = Path::new(OUTPUT_DIRECTORY);
    fs::create_dir_all(directory)?;
    let tabs = vec![
        TabItem::new(1, "https://bank.example.com"),
        TabItem::new_special(2, "Bookmarks", TabKind::Bookmarks),
        TabItem::new_special(3, "Settings", TabKind::Settings),
    ];
    let bookmarks = [
        ("Everyday banking", "https://bank.example.com"),
        ("Savings account", "https://savings.example.com"),
        ("Household bills", "https://bills.example.com"),
        ("Credit union", "https://credit.example.com"),
        ("Travel account", "https://travel.example.com"),
        ("Insurance", "https://insurance.example.com"),
    ]
    .into_iter()
    .map(|(title, url)| Bookmark::new(title, url, BookmarkCategory::General))
    .collect::<Result<Vec<_>, _>>()?;

    write_page(
        directory,
        "chrome.html",
        generate_browser_chrome_html_with_session(&tabs, 2, true, false),
    )?;
    write_page(
        directory,
        "bookmarks.html",
        generate_bookmarks_page_html(&bookmarks),
    )?;
    write_page(
        directory,
        "settings.html",
        with_sample_script(
            generate_settings_page_html_with_session(true, false, true),
            SAMPLE_PERMISSIONS,
        ),
    )?;
    write_page(
        directory,
        "permission-prompt.html",
        with_sample_script(generate_permission_prompt_html(), SAMPLE_REQUEST),
    )?;
    write_page(directory, "keyboard.html", generate_virtual_keyboard_html())?;
    write_page(
        directory,
        "capture-warning.html",
        generate_capture_warning_html(),
    )?;
    write_page(
        directory,
        "taskbar.html",
        generate_desktop_shell_html_with_session(true, false),
    )?;
    fs::write(directory.join("index.html"), PREVIEW_INDEX)?;
    fs::write(directory.join("permissions-qa.html"), PERMISSION_PREVIEW)?;
    if std::env::args().any(|argument| argument == "--capture-permissions") {
        capture::permission_surfaces(directory)?;
    }
    println!(
        "UI preview exported to {}",
        directory.canonicalize()?.display()
    );
    Ok(())
}

/// Adds deterministic visual fixtures without touching the application's policy store.
fn with_sample_script(html: String, script: &str) -> String {
    html.replacen("</body>", &format!("<script>{script}</script></body>"), 1)
}

const SAMPLE_PERMISSIONS: &str = r#"
window.safeBrowsePreviewPermissions = { version: 1, popup_default: 'ask', site_rules: [
    { origin: 'https://bank.example.com', permission: 'popups', decision: 'ask' },
    { origin: 'https://appointments.example.com:8443', permission: 'camera', decision: 'allow' },
    { origin: 'https://media.example.com', permission: 'autoplay', decision: 'block' },
    { origin: 'https://design.example.com', permission: 'local_fonts', decision: 'allow' }
] };
window.updatePermissions(window.safeBrowsePreviewPermissions);
"#;

const SAMPLE_REQUEST: &str = r#"
window.showRequest({ id: 42, tab_id: 1, permission: 'popups', origin: 'https://bank.example.com',
    requesting_url: 'https://bank.example.com/accounts', user_initiated: true,
    target_url: 'https://identity.example.com/authorize?return_to=https%3A%2F%2Fbank.example.com%2Faccounts&purpose=sample-layout-only' });
"#;

const PERMISSION_PREVIEW: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>SafeBrowse permission layout preview</title>
<style>
* { box-sizing: border-box; } body { margin: 0; padding: 24px; background: #e6e6e6; color: #242424; font: 13px/1.5 'Segoe UI', sans-serif; }
h1 { margin: 0 0 8px; font-size: 22px; } p { margin: 0 0 12px; } h2 { margin: 24px 0 8px; font-size: 16px; }
iframe { display: block; border: 0; background: white; box-shadow: 0 2px 8px #0002; } #prompt { width: 460px; height: 350px; } #settings { width: 800px; height: 600px; }
.controls { display: flex; gap: 8px; margin: 10px 0; } button { font: inherit; padding: 5px 10px; } #result { min-height: 20px; overflow-wrap: anywhere; }
</style></head><body>
<h1>Permission interface preview</h1><p>Sample data only. These controls do not open websites, grant permissions, or save application settings.</p>
<h2>Website request · 460 × 350</h2><div class="controls"><button id="sample-popup">Popup request</button><button id="sample-camera">Camera request</button><button id="sample-long">Long website address</button></div>
<iframe id="prompt" src="permission-prompt.html" title="Sample website permission request"></iframe><p id="result" role="status"></p>
<h2>Settings · 800 × 600</h2><iframe id="settings" src="settings.html" title="Sample permission settings"></iframe>
<script>
const prompt = document.getElementById('prompt');
const settings = document.getElementById('settings');
let nextId = 42;
function showSample(permission, origin, target = null) {
    prompt.contentWindow.showRequest?.({ id: ++nextId, tab_id: 1, permission, origin, target_url: target, requesting_url: origin, user_initiated: true });
    document.getElementById('result').textContent = '';
}
document.getElementById('sample-popup').addEventListener('click', () => showSample('popups', 'https://bank.example.com', 'https://identity.example.com/authorize?sample=true'));
document.getElementById('sample-camera').addEventListener('click', () => showSample('camera', 'https://appointments.example.com:8443'));
document.getElementById('sample-long').addEventListener('click', () => showSample('popups', 'https://' + 'long-subdomain-'.repeat(3) + 'accounts.example.com:8443', 'https://identity.example.com/authorize?return_to=' + 'very-long-sample-value'.repeat(24)));
window.addEventListener('message', event => {
    if (!event.data?.safeBrowsePreview || ![prompt.contentWindow, settings.contentWindow].includes(event.source)) return;
    const command = event.data.command;
    if (command.type === 'UI_READY') return;
    document.getElementById('result').textContent = 'Preview command only: ' + JSON.stringify(command);
    if (command.type === 'RESOLVE_SITE_REQUEST') prompt.contentWindow.showRequestError?.('Preview only. No website permission was changed.');
    const sample = settings.contentWindow.safeBrowsePreviewPermissions;
    if (!sample) return;
    if (command.type === 'SET_POPUP_POLICY') sample.popup_default = command.decision;
    if (['RESET_SITE_PERMISSION', 'SET_SITE_PERMISSION'].includes(command.type)) {
        let origin;
        try { origin = new URL(command.origin).origin; }
        catch { settings.contentWindow.showStatus?.('Preview only: enter a valid website address.', true); return; }
        sample.site_rules = sample.site_rules.filter(rule => rule.origin !== origin || rule.permission !== command.permission);
        if (command.type === 'SET_SITE_PERMISSION') sample.site_rules.push({ origin, permission: command.permission, decision: command.decision });
    }
    if (['SET_POPUP_POLICY', 'RESET_SITE_PERMISSION', 'SET_SITE_PERMISSION'].includes(command.type)) settings.contentWindow.updatePermissions(sample);
});
</script></body></html>"#;

/// Optional rendering fixture: hidden WebView2 surfaces with local sample HTML only.
/// It never creates a SafeBrowse session or reads/writes real website permissions.
mod capture {
    use super::*;
    use base64::Engine;
    use safebrowse::browser::{ProfileManager, ProfileMode};
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};
    use tao::dpi::LogicalSize;
    use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::platform::windows::EventLoopBuilderExtWindows;
    use tao::window::WindowBuilder;
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use wry::{WebContext, WebView, WebViewBuilder, WebViewExtWindows};

    const RENDER_TIMEOUT: Duration = Duration::from_secs(20);

    pub(super) fn permission_surfaces(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let profile = ProfileManager::new(ProfileMode::Ephemeral)?;
        let mut event_loop = EventLoopBuilder::<()>::with_user_event()
            .with_any_thread(true)
            .build();
        let result = (|| {
            let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
            for (filename, width, height) in [
                ("permission-prompt", 460.0, 350.0),
                ("settings", 800.0, 600.0),
            ] {
                let window = WindowBuilder::new()
                    .with_visible(false)
                    .with_inner_size(LogicalSize::new(width, height))
                    .build(&event_loop)?;
                let (sender, receiver) = mpsc::channel();
                let proxy = event_loop.create_proxy();
                let view = WebViewBuilder::new_with_web_context(&mut context)
                    .with_html(fs::read_to_string(
                        directory.join(format!("{filename}.html")),
                    )?)
                    .with_visible(true)
                    .with_bounds(wry::Rect {
                        position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
                        size: wry::dpi::LogicalSize::new(width, height).into(),
                    })
                    .with_permission_handler(|_| wry::PermissionResponse::Deny)
                    .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                    .with_on_page_load_handler(move |event, _| {
                        if matches!(event, wry::PageLoadEvent::Finished) {
                            let _ = sender.send(());
                            let _ = proxy.send_event(());
                        }
                    })
                    .build_as_child(&window)?;
                receive(&mut event_loop, receiver)?;
                protocol(
                    &mut event_loop,
                    &view,
                    "Emulation.setDeviceMetricsOverride",
                    &format!(
                        r#"{{"width":{width},"height":{height},"deviceScaleFactor":1,"mobile":false}}"#
                    ),
                )?;
                screenshot(
                    &mut event_loop,
                    &view,
                    &directory.join(format!("{filename}-preview.png")),
                )?;
                if filename == "settings" {
                    evaluate(&mut event_loop, &view, "document.getElementById('permissions-heading').scrollIntoView();document.getElementById('site-rules').getBoundingClientRect().width")?;
                    screenshot(
                        &mut event_loop,
                        &view,
                        &directory.join("settings-permissions-preview.png"),
                    )?;
                } else {
                    evaluate(&mut event_loop, &view, "window.showRequest({id:99,permission:'popups',origin:'https://'+'long-subdomain-'.repeat(3)+'accounts.example.com:8443',target_url:'https://identity.example.com/authorize?return_to='+'very-long-sample-value'.repeat(24)});document.body.getBoundingClientRect().height")?;
                    screenshot(
                        &mut event_loop,
                        &view,
                        &directory.join("permission-prompt-long-preview.png"),
                    )?;
                }
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        drop(event_loop);
        profile.purge_ephemeral_storage()?;
        result
    }

    fn receive<T>(
        event_loop: &mut EventLoop<()>,
        receiver: Receiver<T>,
    ) -> Result<T, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + RENDER_TIMEOUT;
        let mut result = None;
        event_loop.run_return(|_, _, flow| {
            *flow = ControlFlow::WaitUntil(deadline);
            if let Ok(value) = receiver.try_recv() {
                result = Some(value);
            }
            if result.is_some() || Instant::now() >= deadline {
                *flow = ControlFlow::Exit;
            }
        });
        result.ok_or_else(|| "Timed out rendering the local UI preview".into())
    }

    fn evaluate(
        event_loop: &mut EventLoop<()>,
        view: &WebView,
        expression: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::channel();
        let proxy = event_loop.create_proxy();
        view.evaluate_script_with_callback(expression, move |result| {
            let _ = sender.send(result);
            let _ = proxy.send_event(());
        })?;
        receive(event_loop, receiver)
    }

    fn protocol(
        event_loop: &mut EventLoop<()>,
        view: &WebView,
        method: &str,
        arguments: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::channel();
        let proxy = event_loop.create_proxy();
        unsafe {
            view.webview().CallDevToolsProtocolMethod(
                &webview2_core::HSTRING::from(method),
                &webview2_core::HSTRING::from(arguments),
                &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    move |result, response| {
                        let _ = sender
                            .send(result.map(|()| response).map_err(|error| error.to_string()));
                        let _ = proxy.send_event(());
                        Ok(())
                    },
                )),
            )
        }?;
        receive(event_loop, receiver)?.map_err(Into::into)
    }

    fn screenshot(
        event_loop: &mut EventLoop<()>,
        view: &WebView,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = protocol(
            event_loop,
            view,
            "Page.captureScreenshot",
            r#"{"format":"png","captureBeyondViewport":false}"#,
        )?;
        let payload: serde_json::Value = serde_json::from_str(&response)?;
        let image = base64::engine::general_purpose::STANDARD.decode(
            payload["data"]
                .as_str()
                .ok_or("Screenshot response had no image")?,
        )?;
        fs::write(path, image)?;
        Ok(())
    }
}

const PREVIEW_INDEX: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>SafeBrowse UI preview</title><style>
* { box-sizing: border-box; } html, body { width: 100%; height: 100%; margin: 0; background: #f7f7f7; }
body { display: grid; grid-template-rows: 110px minmax(0, 1fr) 0 46px; }
body.keyboard-open { grid-template-rows: 110px minmax(0, 1fr) 230px 46px; }
iframe { width: 100%; height: 100%; border: 0; display: block; min-height: 0; }
#keyboard { visibility: hidden; } .keyboard-open #keyboard { visibility: visible; }
#warning { position: fixed; inset: 0; z-index: 5; width: 100%; height: 100%; }
#warning[hidden] { display: none; }
</style></head><body>
<iframe id="chrome" src="chrome.html" title="Browser controls"></iframe>
<iframe id="content" src="bookmarks.html" title="Browser page"></iframe>
<iframe id="keyboard" src="keyboard.html" title="On-screen keyboard"></iframe>
<iframe id="taskbar" src="taskbar.html" title="Session status"></iframe>
<iframe id="warning" src="capture-warning.html" title="Screen recording warning"></iframe>
<script>
'use strict';
const chromeFrame = document.getElementById('chrome');
const contentFrame = document.getElementById('content');
let activeId = 2;
let keyboardOpen = false;
let nextTabId = 4;
let tabs = [
    { id: 1, title: 'Example Bank', url: 'https://bank.example.com', kind: 'Web', is_secure: true, is_loading: false },
    { id: 2, title: 'Bookmarks', url: 'safebrowse://bookmarks', kind: 'Bookmarks', is_secure: true, is_loading: false },
    { id: 3, title: 'Settings', url: 'safebrowse://settings', kind: 'Settings', is_secure: true, is_loading: false }
];
function syncChrome() {
    chromeFrame.contentWindow.updateTabs?.(tabs, activeId);
    chromeFrame.contentWindow.setMaximizedState?.(true);
    chromeFrame.contentWindow.setOskActive?.(keyboardOpen);
    chromeFrame.contentWindow.setNavigationState?.(false, false);
}
function showPreviewStatus(text, isError = false) { chromeFrame.contentWindow.showStatus?.(text, isError); }
function selectTab(id) {
    const selected = tabs.find(tab => tab.id === id);
    if (!selected) return;
    activeId = id;
    contentFrame.src = selected.kind === 'Settings' ? 'settings.html' : 'bookmarks.html';
    syncChrome();
    if (selected.kind === 'Web') showPreviewStatus('Layout preview: external websites are not loaded.');
}
window.addEventListener('message', event => {
    if (!event.data?.safeBrowsePreview) return;
    const frames = ['chrome', 'content', 'keyboard', 'taskbar', 'warning'];
    if (!frames.some(id => document.getElementById(id).contentWindow === event.source)) return;
    const command = event.data.command;
    if (command.type === 'UI_READY') { syncChrome(); return; }
    if (command.type === 'ACKNOWLEDGE_CAPTURE_RISK') { document.getElementById('warning').hidden = true; return; }
    if (command.type === 'OPEN_SETTINGS') { selectTab(3); return; }
    if (command.type === 'OPEN_BOOKMARKS') { selectTab(2); return; }
    if (command.type === 'SWITCH_TAB') { selectTab(command.id); return; }
    if (command.type === 'TOGGLE_OSK') {
        keyboardOpen = !keyboardOpen;
        document.body.classList.toggle('keyboard-open', keyboardOpen);
        syncChrome();
        return;
    }
    if (command.type === 'KEY_INPUT') { chromeFrame.contentWindow.injectOmniboxKey?.(command.action); return; }
    if (command.type === 'NAVIGATE') { showPreviewStatus('Layout preview only: ' + command.url); return; }
    if (command.type === 'NEW_TAB') {
        const id = nextTabId++;
        tabs.push({ id, title: 'New tab', url: 'https://example.com', kind: 'Web', is_secure: true, is_loading: false });
        selectTab(id);
        return;
    }
    if (command.type === 'CLOSE_TAB') {
        if (tabs.length <= 1) { showPreviewStatus('Layout preview: session controls are inactive.'); return; }
        tabs = tabs.filter(tab => tab.id !== command.id);
        if (activeId === command.id) selectTab(tabs[tabs.length - 1].id); else syncChrome();
        return;
    }
    if (command.type === 'ADD_BOOKMARK_FROM_DATA' || command.type === 'REMOVE_BOOKMARK') {
        contentFrame.contentWindow.showBookmarkStatus?.('Layout preview: bookmarks are not saved.', true);
        return;
    }
    if (['SET_INPUT_TARGET', 'QUERY_BATTERY', 'START_DRAG'].includes(command.type)) return;
    showPreviewStatus('Layout preview: native session controls are inactive.');
});
</script></body></html>"#;
