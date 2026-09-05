use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event_loop::EventLoopBuilder;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::WindowBuilder;
use wry::dpi::{Position, Size};
use wry::{Rect, WebContext, WebViewBuilder};

#[test]
fn test_shell_and_browser_windows_creation() {
    let event_loop = EventLoopBuilder::new().with_any_thread(true).build();

    let shell_win = WindowBuilder::new()
        .with_visible(false)
        .with_inner_size(LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)
        .expect("build shell window");

    let browser_win = WindowBuilder::new()
        .with_visible(false)
        .with_inner_size(LogicalSize::new(800.0, 600.0))
        .build(&event_loop)
        .expect("build browser window");

    let mut context = WebContext::new(None);

    let shell_webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_html("<html><body>Shell Desktop</body></html>")
        .build(&shell_win);
    assert!(
        shell_webview.is_ok(),
        "shell webview: {:?}",
        shell_webview.err()
    );

    let top_bounds = Rect {
        position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
        size: Size::Logical(LogicalSize::new(800.0, 108.0)),
    };
    let content_bounds = Rect {
        position: Position::Logical(LogicalPosition::new(0.0, 108.0)),
        size: Size::Logical(LogicalSize::new(800.0, 492.0)),
    };

    let browser_chrome = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(top_bounds)
        .with_html("<html><body>Browser Chrome</body></html>")
        .build_as_child(&browser_win);
    assert!(
        browser_chrome.is_ok(),
        "browser chrome: {:?}",
        browser_chrome.err()
    );

    let browser_content = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(content_bounds)
        .with_html("<html><body>Browser Content</body></html>")
        .build_as_child(&browser_win);
    assert!(
        browser_content.is_ok(),
        "browser content: {:?}",
        browser_content.err()
    );
}
