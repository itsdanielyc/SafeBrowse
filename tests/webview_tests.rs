use tao::event_loop::EventLoopBuilder;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::WindowBuilder;
use wry::{Rect, WebContext, WebViewBuilder};
use wry::dpi::{LogicalPosition, LogicalSize, Position, Size};

#[test]
fn test_multiple_webviews_on_window() {
    let event_loop = EventLoopBuilder::new().with_any_thread(true).build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .with_inner_size(LogicalSize::new(800.0, 600.0))
        .build(&event_loop)
        .expect("build window");

    let mut context = WebContext::new(None);

    let top_bounds = Rect {
        position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
        size: Size::Logical(LogicalSize::new(800.0, 110.0)),
    };
    let content_bounds = Rect {
        position: Position::Logical(LogicalPosition::new(0.0, 110.0)),
        size: Size::Logical(LogicalSize::new(800.0, 490.0)),
    };

    let webview1 = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(top_bounds)
        .with_html("<html><body><h1>Header</h1></body></html>")
        .build(&window);

    assert!(webview1.is_ok(), "webview1 creation: {:?}", webview1.err());

    let webview2 = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(content_bounds)
        .with_html("<html><body><p>Content</p></body></html>")
        .build(&window);

    assert!(webview2.is_ok(), "webview2 creation: {:?}", webview2.err());
}
