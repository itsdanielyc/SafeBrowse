//! Exercises native child-view geometry and loaded documents, not just construction.

use std::time::{Duration, Instant};

use safebrowse::browser::{ProfileManager, ProfileMode};
use safebrowse::keyboard::VirtualKeyboard;
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::{Window, WindowBuilder};
use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

const NATIVE_EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const INITIAL_WIDTH: f64 = 800.0;
const INITIAL_HEIGHT: f64 = 600.0;
const HEADER_HEIGHT: f64 = 110.0;

#[derive(Debug)]
enum NativeTestEvent {
    Loaded(&'static str),
    Evaluated(String),
}

/// Pumps the actual Windows event loop until the requested observation arrives.
fn wait_for_observation<T>(
    event_loop: &mut EventLoop<NativeTestEvent>,
    description: &str,
    mut observe: impl FnMut(Event<NativeTestEvent>) -> Option<T>,
) -> T {
    let deadline = Instant::now() + NATIVE_EVENT_TIMEOUT;
    let mut observation = None;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(deadline);
        if observation.is_none() {
            observation = observe(event);
        }
        if observation.is_some() || Instant::now() >= deadline {
            *control_flow = ControlFlow::Exit;
        }
    });
    observation.unwrap_or_else(|| panic!("Timed out waiting for {description}"))
}

fn logical_bounds(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

/// Mirrors the application's initially hidden, explicitly positioned native surfaces.
fn create_child_view(
    window: &Window,
    context: &mut WebContext,
    proxy: EventLoopProxy<NativeTestEvent>,
    name: &'static str,
    bounds: Rect,
) -> WebView {
    WebViewBuilder::new_with_web_context(context)
        .with_bounds(bounds)
        .with_visible(false)
        .with_html(format!(
            "<!doctype html><html><head><style>body {{ margin: 0; background: rgb(244, 244, 244); }}</style></head><body><h1>{name}</h1></body></html>"
        ))
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = proxy.send_event(NativeTestEvent::Loaded(name));
            }
        })
        .build_as_child(window)
        .unwrap_or_else(|error| panic!("Cannot create {name} child view: {error}"))
}

/// Compares physical pixels to retain correctness under Windows display scaling.
fn assert_child_bounds(view: &WebView, expected: Rect, scale_factor: f64) {
    let actual = view.bounds().expect("read native child bounds");
    assert_eq!(
        actual.position.to_physical::<i32>(scale_factor),
        expected.position.to_physical::<i32>(scale_factor),
        "child position changed without an explicit layout update"
    );
    assert_eq!(
        actual.size.to_physical::<u32>(scale_factor),
        expected.size.to_physical::<u32>(scale_factor),
        "child size changed without an explicit layout update"
    );
}

fn inspect_document(
    event_loop: &mut EventLoop<NativeTestEvent>,
    view: &WebView,
) -> serde_json::Value {
    evaluate_document(
        event_loop,
        view,
        "({ text: document.body.innerText.trim(), width: innerWidth, height: innerHeight, background: getComputedStyle(document.body).backgroundColor })",
    )
}

/// Runs a DOM observation in the actual WebView2 renderer and waits for its result.
fn evaluate_document(
    event_loop: &mut EventLoop<NativeTestEvent>,
    view: &WebView,
    script: &str,
) -> serde_json::Value {
    let proxy = event_loop.create_proxy();
    view.evaluate_script_with_callback(script, move |result| {
        let _ = proxy.send_event(NativeTestEvent::Evaluated(result));
    })
    .expect("evaluate the loaded native document");
    let serialized =
        wait_for_observation(
            event_loop,
            "native document evaluation",
            |event| match event {
                Event::UserEvent(NativeTestEvent::Evaluated(result)) => Some(result),
                _ => None,
            },
        );
    serde_json::from_str(&serialized).expect("WebView2 returned document inspection JSON")
}

/// Applies a real virtual key to the remembered editor and reports its focus and caret.
fn inspect_virtual_editor_action(
    event_loop: &mut EventLoop<NativeTestEvent>,
    view: &WebView,
    action: &str,
) -> serde_json::Value {
    evaluate_document(
        event_loop,
        view,
        &format!(
            r#"(() => {{
                {}
                const editor = window.__safebrowse_last_input;
                const selection = document.getSelection();
                const caretRange = document.createRange();
                caretRange.selectNodeContents(editor);
                caretRange.setEnd(selection.anchorNode, selection.anchorOffset);
                return {{ focused: document.activeElement.id, value: editor.textContent,
                    html: editor.innerHTML, caret: caretRange.toString().length }};
            }})()"#,
            VirtualKeyboard::generate_dom_injection_script(action),
        ),
    )
}

/// Virtual-key callbacks must not refocus a field after a subsequent control click.
fn assert_virtual_input_preserves_control_focus(
    event_loop: &mut EventLoop<NativeTestEvent>,
    view: &WebView,
) {
    let field_script = format!(
        r#"(() => {{
            const field = document.createElement('input');
            field.value = 'before after';
            const control = document.createElement('button');
            control.id = 'minimize-control';
            control.textContent = 'Minimize';
            document.body.replaceChildren(field, control);
            field.focus();
            field.setSelectionRange(7, 7);
            window.__safebrowse_last_input = field;
            control.focus();
            const beforeFocus = document.activeElement.id;
            {}
            return {{ beforeFocus, focused: document.activeElement.id,
                value: field.value, caret: field.selectionStart }};
        }})()"#,
        VirtualKeyboard::generate_dom_injection_script("X"),
    );
    let field = evaluate_document(event_loop, view, &field_script);
    assert_eq!(field["beforeFocus"], "minimize-control");
    assert_eq!(field["focused"], "minimize-control");
    assert_eq!(field["value"], "before Xafter");
    assert_eq!(field["caret"], 8);

    let editor_script = format!(
        r#"(() => {{
            const editor = document.createElement('div');
            editor.contentEditable = 'true';
            editor.textContent = 'before after';
            const control = document.createElement('button');
            control.id = 'minimize-control';
            control.textContent = 'Minimize';
            document.body.replaceChildren(editor, control);
            editor.focus();
            const range = document.createRange();
            range.setStart(editor.firstChild, 7);
            range.collapse(true);
            const selection = document.getSelection();
            selection.removeAllRanges();
            selection.addRange(range);
            window.__safebrowse_last_input = editor;
            control.focus();
            const beforeFocus = document.activeElement.id;
            {}
            const caretRange = document.createRange();
            caretRange.selectNodeContents(editor);
            caretRange.setEnd(selection.anchorNode, selection.anchorOffset);
            return {{ beforeFocus, focused: document.activeElement.id,
                value: editor.textContent, caret: caretRange.toString().length }};
        }})()"#,
        VirtualKeyboard::generate_dom_injection_script("X"),
    );
    let editor = evaluate_document(event_loop, view, &editor_script);
    assert_eq!(editor["beforeFocus"], "minimize-control");
    assert_eq!(editor["focused"], "minimize-control");
    assert_eq!(editor["value"], "before Xafter");
    assert_eq!(editor["caret"], 8);

    let deleted = inspect_virtual_editor_action(event_loop, view, "BACKSPACE");
    assert_eq!(deleted["focused"], "minimize-control");
    assert_eq!(deleted["value"], "before after");
    assert_eq!(deleted["caret"], 7);
    let line_break = inspect_virtual_editor_action(event_loop, view, "ENTER");
    assert_eq!(line_break["focused"], "minimize-control");
    assert_eq!(line_break["html"], "before <br>after");
    let second_line = inspect_virtual_editor_action(event_loop, view, "Y");
    assert_eq!(second_line["focused"], "minimize-control");
    assert_eq!(second_line["html"], "before <br>Yafter");
    inspect_virtual_editor_action(event_loop, view, "BACKSPACE");
    let joined_line = inspect_virtual_editor_action(event_loop, view, "BACKSPACE");
    assert_eq!(joined_line["focused"], "minimize-control");
    assert_eq!(joined_line["html"], "before after");

    evaluate_document(
        event_loop,
        view,
        "window.__safebrowse_last_input.addEventListener('beforeinput', event => event.preventDefault(), { once: true }); null",
    );
    let canceled = inspect_virtual_editor_action(event_loop, view, "X");
    assert_eq!(canceled["focused"], "minimize-control");
    assert_eq!(canceled["value"], "before after");

    evaluate_document(
        event_loop,
        view,
        r#"(() => {
            const editor = window.__safebrowse_last_input;
            editor.innerHTML = '<span>a👩</span><span>‍💻</span><span>b</span>';
            editor.focus();
            const range = document.createRange();
            range.setStart(editor.lastChild.firstChild, 0);
            range.collapse(true);
            const selection = document.getSelection();
            selection.removeAllRanges();
            selection.addRange(range);
            document.getElementById('minimize-control').focus();
            return null;
        })()"#,
    );
    let grapheme_deleted = inspect_virtual_editor_action(event_loop, view, "BACKSPACE");
    assert_eq!(grapheme_deleted["focused"], "minimize-control");
    assert_eq!(grapheme_deleted["value"], "ab");
    assert_eq!(grapheme_deleted["caret"], 1);
}

/// CSS viewport dimensions round outward after native pixels are scaled at fractional DPI.
fn assert_document_dimension(document: &serde_json::Value, dimension: &str, expected: f64) {
    let actual = document[dimension]
        .as_f64()
        .expect("document viewport dimension is numeric");
    assert!(
        (actual - expected).abs() <= 1.0,
        "document {dimension} is {actual}, expected approximately {expected}"
    );
}

fn resize_parent(
    event_loop: &mut EventLoop<NativeTestEvent>,
    window: &Window,
    size: LogicalSize<f64>,
) {
    let expected = size.to_physical::<u32>(window.scale_factor());
    window.set_inner_size(size);
    wait_for_observation(event_loop, "parent window resize", |event| match event {
        Event::WindowEvent {
            window_id,
            event: WindowEvent::Resized(actual),
            ..
        } if window_id == window.id() && actual == expected => Some(()),
        _ => None,
    });
}

#[test]
fn child_views_load_and_keep_their_bounds_after_resize_and_sibling_drop() {
    let profile =
        ProfileManager::new(ProfileMode::Ephemeral).expect("create isolated test profile");
    let mut event_loop = EventLoopBuilder::<NativeTestEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .with_inner_size(LogicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT))
        .build(&event_loop)
        .expect("build hidden test window");
    let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
    let header_bounds = logical_bounds(0.0, 0.0, INITIAL_WIDTH, HEADER_HEIGHT);
    let content_bounds = logical_bounds(
        0.0,
        HEADER_HEIGHT,
        INITIAL_WIDTH,
        INITIAL_HEIGHT - HEADER_HEIGHT,
    );
    let header = create_child_view(
        &window,
        &mut context,
        event_loop.create_proxy(),
        "Header",
        header_bounds,
    );
    let content = create_child_view(
        &window,
        &mut context,
        event_loop.create_proxy(),
        "Content",
        content_bounds,
    );
    header.set_visible(true).expect("show header controller");
    content.set_visible(true).expect("show content controller");
    let mut header_loaded = false;
    let mut content_loaded = false;
    wait_for_observation(
        &mut event_loop,
        "both native documents to finish loading",
        |event| {
            match event {
                Event::UserEvent(NativeTestEvent::Loaded("Header")) => header_loaded = true,
                Event::UserEvent(NativeTestEvent::Loaded("Content")) => content_loaded = true,
                _ => {}
            }
            (header_loaded && content_loaded).then_some(())
        },
    );

    let header_document = inspect_document(&mut event_loop, &header);
    assert_eq!(header_document["text"], "Header");
    assert_document_dimension(&header_document, "width", INITIAL_WIDTH);
    assert_document_dimension(&header_document, "height", HEADER_HEIGHT);
    assert_eq!(header_document["background"], "rgb(244, 244, 244)");
    let mut visible = false.into();
    unsafe { header.controller().IsVisible(&mut visible) }.expect("read controller visibility");
    assert!(visible.as_bool(), "header controller remained hidden");

    resize_parent(&mut event_loop, &window, LogicalSize::new(1000.0, 720.0));
    assert_child_bounds(&header, header_bounds, window.scale_factor());
    assert_child_bounds(&content, content_bounds, window.scale_factor());

    drop(header);
    resize_parent(&mut event_loop, &window, LogicalSize::new(900.0, 680.0));
    assert_child_bounds(&content, content_bounds, window.scale_factor());
    let content_document = inspect_document(&mut event_loop, &content);
    assert_eq!(content_document["text"], "Content");
    assert_document_dimension(&content_document, "width", INITIAL_WIDTH);
    assert_document_dimension(&content_document, "height", INITIAL_HEIGHT - HEADER_HEIGHT);
    assert_virtual_input_preserves_control_focus(&mut event_loop, &content);

    drop(content);
    drop(context);
    drop(window);
    drop(event_loop);
    profile
        .purge_ephemeral_storage()
        .expect("clean native test profile");
}
