//! Bundled brand artwork shared by the trusted shell and native Windows windows.

use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::LazyLock;
use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows};
use tao::window::{BadIcon, Icon, WindowBuilder};

const APPLICATION_ICON_RESOURCE: u16 = 1;
const MARK_PLACEHOLDER: &str = "__BRAND_MARK__";
const WORDMARK_PLACEHOLDER: &str = "__BRAND_WORDMARK__";

static MARK_DATA_URL: LazyLock<String> =
    LazyLock::new(|| png_data_url(include_bytes!("../../assets/branding/safebrowse-mark.png")));
static WORDMARK_DATA_URL: LazyLock<String> = LazyLock::new(|| {
    png_data_url(include_bytes!(
        "../../assets/branding/safebrowse-wordmark.png"
    ))
});

/// Encodes trusted PNG bytes for the shell's existing data-only image policy.
fn png_data_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

/// Expands only static artwork tokens, before any presentation data is inserted.
pub(super) fn embed_images(template: &str) -> String {
    let mut html = template.to_owned();
    if html.contains(MARK_PLACEHOLDER) {
        html = html.replace(MARK_PLACEHOLDER, MARK_DATA_URL.as_str());
    }
    if html.contains(WORDMARK_PLACEHOLDER) {
        html = html.replace(WORDMARK_PLACEHOLDER, WORDMARK_DATA_URL.as_str());
    }
    html
}

/// Applies the embedded colour icon to both the native caption and Windows taskbar.
pub(crate) fn window_builder() -> Result<WindowBuilder, BadIcon> {
    let icon = Icon::from_resource(APPLICATION_ICON_RESOURCE, None)?;
    Ok(WindowBuilder::new()
        .with_window_icon(Some(icon.clone()))
        .with_taskbar_icon(Some(icon)))
}
