//! Local, network-independent user interface assets for SafeBrowse.
//!
//! Templates contain no untrusted HTML. Application data is encoded as script-safe
//! JSON, then rendered using DOM text nodes inside the trusted shell WebViews.

use crate::bookmarks::Bookmark;
use crate::browser::tabs::TabItem;
use serde::Serialize;

const COMMON_CSS: &str = include_str!("web/common.css");
const COMMON_JAVASCRIPT: &str = include_str!("web/common.js");
const UNKNOWN_BATTERY_PERCENTAGE: u8 = 255;
const LOCAL_CONTENT_POLICY: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'">"#;

/// Data displayed about the running session; never changes security policy.
#[derive(Serialize)]
struct SessionPresentation {
    capture_allowed: bool,
    is_isolated: bool,
    temporary_profile: bool,
    version: &'static str,
}

impl SessionPresentation {
    fn new(capture_allowed: bool, is_isolated: bool, temporary_profile: bool) -> Self {
        Self {
            capture_allowed,
            is_isolated,
            temporary_profile,
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

/// Encodes JSON for an HTML script element without permitting script termination.
///
/// HTML parsers recognize `</script>` even inside a JSON string. Escaping `<`
/// prevents bookmark titles, website URLs, and page titles from becoming markup.
fn script_json(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .expect("UI presentation data must be serializable")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Expands trusted shared assets before inserting the serialized presentation data.
fn render_template(template: &str, presentation: &impl Serialize) -> String {
    super::branding::embed_images(template)
        .replacen("<head>", &format!("<head>{LOCAL_CONTENT_POLICY}"), 1)
        .replace("__COMMON_CSS__", COMMON_CSS)
        .replace("__COMMON_JAVASCRIPT__", COMMON_JAVASCRIPT)
        .replace("__PRESENTATION_JSON__", &script_json(presentation))
}

/// Queries system power; a percentage of 255 means no battery reading is available.
///
/// The SVG is a fixed local asset, never supplied by a website or external input.
pub fn get_system_battery_status() -> (String, u8, bool) {
    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }
    type GetSystemPowerStatusFn = unsafe extern "system" fn(*mut SystemPowerStatus) -> i32;
    const BATTERY_ICON: &str = r#"<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><rect x="2" y="7" width="17" height="10" rx="2"/><path d="M22 10v4M5 10v4M8 10v4M11 10v4"/></svg>"#;
    const NO_SYSTEM_BATTERY: u8 = 128;

    // kernel32 remains mapped for the process lifetime; no DLL reference is acquired.
    unsafe {
        if let Ok(module) = windows::Win32::System::LibraryLoader::GetModuleHandleA(
            windows::core::s!("kernel32.dll"),
        ) {
            if let Some(address) = windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::s!("GetSystemPowerStatus"),
            ) {
                let query_power: GetSystemPowerStatusFn = std::mem::transmute(address);
                let mut status = SystemPowerStatus {
                    ac_line_status: 255,
                    battery_flag: 255,
                    battery_life_percent: UNKNOWN_BATTERY_PERCENTAGE,
                    system_status_flag: 0,
                    battery_life_time: 0,
                    battery_full_life_time: 0,
                };
                if query_power(&mut status) != 0 {
                    let percentage = if status.battery_flag != NO_SYSTEM_BATTERY
                        && status.battery_life_percent <= 100
                    {
                        status.battery_life_percent
                    } else {
                        UNKNOWN_BATTERY_PERCENTAGE
                    };
                    return (
                        BATTERY_ICON.to_owned(),
                        percentage,
                        status.ac_line_status == 1,
                    );
                }
            }
        }
    }
    (BATTERY_ICON.to_owned(), UNKNOWN_BATTERY_PERCENTAGE, false)
}

/// Generates the desktop taskbar for an isolated, capture-protected session.
pub fn generate_desktop_shell_html() -> String {
    generate_desktop_shell_html_with_session(false, true)
}

/// Generates the 46px taskbar with the actual desktop and recording mode.
pub fn generate_desktop_shell_html_with_session(
    capture_allowed: bool,
    is_isolated: bool,
) -> String {
    let (_, percentage, _) = get_system_battery_status();
    render_template(
        include_str!("web/taskbar.html"),
        &serde_json::json!({
            "session": SessionPresentation::new(capture_allowed, is_isolated, true),
            "battery_percentage": percentage,
            "language": crate::keyboard::osk::get_system_input_language(),
        }),
    )
}

/// Generates the 110px browser chrome for a protected, isolated session.
pub fn generate_browser_chrome_html(tabs: &[TabItem], active_id: usize) -> String {
    generate_browser_chrome_html_with_session(tabs, active_id, false, true)
}

/// Generates accessible browser controls with explicit recording-mode status.
pub fn generate_browser_chrome_html_with_session(
    tabs: &[TabItem],
    active_id: usize,
    capture_allowed: bool,
    is_isolated: bool,
) -> String {
    render_template(
        include_str!("web/chrome.html"),
        &serde_json::json!({
            "tabs": tabs,
            "active_id": active_id,
            "session": SessionPresentation::new(capture_allowed, is_isolated, true),
        }),
    )
}

/// Generates the 230px keyboard with separate Shift and Caps Lock behavior.
pub fn generate_virtual_keyboard_html() -> String {
    render_template(include_str!("web/keyboard.html"), &())
}

/// Generates the app-owned picker for installed Windows input layouts.
pub fn generate_language_picker_html() -> String {
    render_template(include_str!("web/language-picker.html"), &())
}

/// Generates the trusted decision surface for native website permission requests.
pub fn generate_permission_prompt_html() -> String {
    render_template(include_str!("web/permission-prompt.html"), &())
}

/// Generates the app-owned confirmation for a single deferred file download.
pub fn generate_download_prompt_html() -> String {
    render_template(include_str!("web/download-prompt.html"), &())
}

/// Generates searchable, editable bookmarks without interpolating HTML attributes.
pub fn generate_bookmarks_page_html(bookmarks: &[Bookmark]) -> String {
    render_template(include_str!("web/bookmarks.html"), &bookmarks)
}

/// Generates settings for a protected, isolated session with a temporary profile.
pub fn generate_settings_page_html() -> String {
    generate_settings_page_html_with_session(false, true, true)
}

/// Describes the active security policy without presenting nonfunctional toggles.
pub fn generate_settings_page_html_with_session(
    capture_allowed: bool,
    is_isolated: bool,
    temporary_profile: bool,
) -> String {
    render_template(
        include_str!("web/settings.html"),
        &SessionPresentation::new(capture_allowed, is_isolated, temporary_profile),
    )
}

/// Generates the blocking red warning shown before a recordable session begins.
///
/// Native code must keep website content unavailable until the explicit
/// `ACKNOWLEDGE_CAPTURE_RISK` event; this HTML does not grant access itself.
pub fn generate_capture_warning_html() -> String {
    render_template(include_str!("web/capture-warning.html"), &())
}

/// Generates the desktop companion with explicit click or keyboard activation.
pub fn generate_dock_companion_html() -> String {
    render_template(include_str!("web/companion.html"), &())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_data_cannot_close_its_script_element() {
        let hostile_title = "</script><img src=x onerror=alert(1)> & \u{2028}";
        let encoded = script_json(&hostile_title);
        assert!(!encoded.contains('<'));
        assert!(!encoded.contains('&'));
        assert!(!encoded.contains('\u{2028}'));
        assert_eq!(
            serde_json::from_str::<String>(&encoded).unwrap(),
            hostile_title
        );
    }

    #[test]
    fn templates_insert_bookmark_data_as_json_and_not_event_attributes() {
        let bookmark = Bookmark::new(
            "</script><script>window.compromised=true</script>",
            "https://example.com/?q='\"<test>",
            crate::bookmarks::BookmarkCategory::General,
        )
        .unwrap();
        let html = generate_bookmarks_page_html(&[bookmark]);
        assert!(!html.contains("</script><script>window.compromised"));
        assert!(!html.contains("onclick=\"openBookmark"));
        assert!(!html.contains("__PRESENTATION_JSON__"));
    }

    #[test]
    fn recording_warning_requires_explicit_acknowledgment() {
        let html = generate_capture_warning_html();
        assert!(html.contains("Screen recording is allowed"));
        assert!(html.contains("If this is a production app, stop using it."));
        assert!(html.contains("ACKNOWLEDGE_CAPTURE_RISK"));
        assert!(html.contains(">OK</button>"));
    }
}
