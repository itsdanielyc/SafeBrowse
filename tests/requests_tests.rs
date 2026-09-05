//! Exercises deferred requests against real WebView2 documents on a loopback fixture server.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use safebrowse::browser::permissions::SitePermission;
use safebrowse::browser::requests::{
    reset_native_permission_decisions, RequestAttachment, RequestBroker, RequestEvent,
    RequestNotice,
};
use safebrowse::browser::{ProfileManager, ProfileMode};
use tao::dpi::LogicalSize;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::{Window, WindowBuilder};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    GetNonDefaultPermissionSettingsCompletedHandler, SetPermissionStateCompletedHandler,
};
use webview2_core::{Interface, HSTRING};
use wry::{
    PageLoadEvent, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
const PARENT_TAB: usize = 1;
const POPUP_TAB: usize = 2;
const BLANK_POPUP_TAB: usize = 3;

#[derive(Debug)]
enum TestEvent {
    Loaded(usize),
    Title(usize, String),
    Evaluated(String),
    Request(RequestEvent),
    ProfileUpdated(Result<(), String>),
    ProfilePermissionCount(u32),
}

struct FixtureServer {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl FixtureServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if worker_stopped.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request_line = String::new();
                if BufReader::new(&stream)
                    .read_line(&mut request_line)
                    .is_err()
                {
                    continue;
                }
                let path = request_line.split_whitespace().nth(1).unwrap_or("/");
                let body = match path {
                    "/popup" => "<!doctype html><title>Popup fixture</title><h1>Popup</h1><script>window.opener.postMessage('popup-ready', location.origin)</script>",
                    "/embedded-popup" => "<!doctype html><title>Embedded fixture</title><script>window.open('/popup')</script>",
                    _ => "<!doctype html><title>Parent fixture</title><h1>Request fixture</h1><script>window.messages=[];addEventListener('message',e=>{if(e.origin===location.origin)window.messages.push(e.data)})</script>",
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            address,
            stopped,
            worker: Some(worker),
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.address)
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin())
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn wait_for<T>(
    event_loop: &mut EventLoop<TestEvent>,
    description: &str,
    mut observe: impl FnMut(TestEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    let mut result = None;
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::WaitUntil(deadline);
        if let Event::UserEvent(event) = event {
            let failure = match &event {
                TestEvent::Request(RequestEvent::Failed { message, .. }) => Some(message.clone()),
                _ => None,
            };
            if result.is_none() {
                result = observe(event);
            }
            if result.is_none() {
                if let Some(message) = failure {
                    panic!("Native request failed while waiting for {description}: {message}");
                }
            }
        }
        if result.is_some() || Instant::now() >= deadline {
            *flow = ControlFlow::Exit;
        }
    });
    result.unwrap_or_else(|| panic!("Timed out waiting for {description}"))
}

fn wait_for_load(event_loop: &mut EventLoop<TestEvent>, tab_id: usize) {
    wait_for(event_loop, "document load", |event| match event {
        TestEvent::Loaded(id) if id == tab_id => Some(()),
        _ => None,
    });
}

fn wait_for_request(
    event_loop: &mut EventLoop<TestEvent>,
    permission: SitePermission,
) -> RequestNotice {
    wait_for(
        event_loop,
        "native permission or popup request",
        |event| match event {
            TestEvent::Request(RequestEvent::Requested(request))
                if request.permission == permission =>
            {
                Some(request)
            }
            _ => None,
        },
    )
}

fn evaluate(
    event_loop: &mut EventLoop<TestEvent>,
    view: &WebView,
    script: &str,
) -> serde_json::Value {
    let proxy = event_loop.create_proxy();
    view.evaluate_script_with_callback(script, move |value| {
        let _ = proxy.send_event(TestEvent::Evaluated(value));
    })
    .unwrap();
    let serialized = wait_for(
        event_loop,
        "native JavaScript observation",
        |event| match event {
            TestEvent::Evaluated(value) => Some(value),
            _ => None,
        },
    );
    serde_json::from_str(&serialized).unwrap()
}

fn make_view(
    event_loop: &EventLoop<TestEvent>,
    window: &Window,
    context: &mut WebContext,
    broker: &RequestBroker,
    tab_id: usize,
    environment: Option<ICoreWebView2Environment>,
) -> (WebView, RequestAttachment) {
    let load_proxy = event_loop.create_proxy();
    let title_proxy = event_loop.create_proxy();
    let mut builder = WebViewBuilder::new_with_web_context(context)
        .with_visible(false)
        .with_clipboard(false)
        .with_navigation_handler(|uri| uri.starts_with("http://127.0.0.1:") || uri == "about:blank")
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = load_proxy.send_event(TestEvent::Loaded(tab_id));
            }
        })
        .with_document_title_changed_handler(move |title| {
            let _ = title_proxy.send_event(TestEvent::Title(tab_id, title));
        });
    if let Some(environment) = environment {
        builder = builder.with_environment(environment);
    }
    let view = builder.build_as_child(window).unwrap();
    let request_proxy = event_loop.create_proxy();
    let attachment = broker
        .attach(&view, tab_id, move |event| {
            let _ = request_proxy.send_event(TestEvent::Request(event));
        })
        .unwrap();
    (view, attachment)
}

fn native_profile(view: &WebView) -> ICoreWebView2Profile4 {
    unsafe {
        view.webview()
            .cast::<ICoreWebView2_13>()
            .unwrap()
            .Profile()
            .unwrap()
            .cast()
            .unwrap()
    }
}

fn permission_count(event_loop: &mut EventLoop<TestEvent>, view: &WebView) -> u32 {
    let proxy = event_loop.create_proxy();
    unsafe {
        native_profile(view)
            .GetNonDefaultPermissionSettings(
                &GetNonDefaultPermissionSettingsCompletedHandler::create(Box::new(
                    move |result, settings| {
                        result?;
                        let mut count = 0;
                        settings.unwrap().Count(&mut count)?;
                        let _ = proxy.send_event(TestEvent::ProfilePermissionCount(count));
                        Ok(())
                    },
                )),
            )
            .unwrap();
    }
    wait_for(
        event_loop,
        "native profile permission count",
        |event| match event {
            TestEvent::ProfilePermissionCount(count) => Some(count),
            _ => None,
        },
    )
}

fn request_location(view: &WebView) {
    view.evaluate_script("navigator.geolocation.getCurrentPosition(()=>document.title='unexpected-location',e=>document.title='denied-'+e.code,{timeout:10000})").unwrap();
}

#[test]
fn native_requests_preserve_popup_opener_deny_permissions_and_cancel_stale_documents() {
    let server = FixtureServer::new();
    let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
    let mut event_loop = EventLoopBuilder::<TestEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .with_focused(false)
        .with_inner_size(LogicalSize::new(800.0, 600.0))
        .build(&event_loop)
        .unwrap();
    let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
    let broker = RequestBroker::new();
    let (parent, parent_attachment) = make_view(
        &event_loop,
        &window,
        &mut context,
        &broker,
        PARENT_TAB,
        None,
    );

    // Earlier releases persisted denials. Startup must restore native defaults before navigation.
    let proxy = event_loop.create_proxy();
    unsafe {
        native_profile(&parent)
            .SetPermissionState(
                COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
                &HSTRING::from(server.origin()),
                COREWEBVIEW2_PERMISSION_STATE_DENY,
                &SetPermissionStateCompletedHandler::create(Box::new(move |result| {
                    let _ = proxy.send_event(TestEvent::ProfileUpdated(
                        result.map_err(|error| error.to_string()),
                    ));
                    Ok(())
                })),
            )
            .unwrap();
    }
    wait_for(
        &mut event_loop,
        "seeded permission denial",
        |event| match event {
            TestEvent::ProfileUpdated(result) => Some(result),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(permission_count(&mut event_loop, &parent), 1);
    let proxy = event_loop.create_proxy();
    reset_native_permission_decisions(&parent, move |result| {
        let _ = proxy.send_event(TestEvent::ProfileUpdated(result));
    })
    .unwrap();
    wait_for(
        &mut event_loop,
        "permission migration",
        |event| match event {
            TestEvent::ProfileUpdated(result) => Some(result),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(permission_count(&mut event_loop, &parent), 0);

    parent.load_url(&server.url("/start")).unwrap();
    wait_for_load(&mut event_loop, PARENT_TAB);
    parent
        .evaluate_script("window.popup=window.open('/popup','native-request-test')")
        .unwrap();
    let popup_request = wait_for_request(&mut event_loop, SitePermission::Popups);
    assert_eq!(popup_request.origin, server.origin());
    assert_eq!(
        popup_request.target_url.as_deref(),
        Some(server.url("/popup").as_str())
    );
    let (popup, popup_attachment) = make_view(
        &event_loop,
        &window,
        &mut context,
        &broker,
        POPUP_TAB,
        Some(broker.popup_environment(popup_request.id).unwrap()),
    );
    broker.resolve_popup(popup_request.id, &popup).unwrap();
    wait_for_load(&mut event_loop, POPUP_TAB);
    assert_eq!(
        evaluate(
            &mut event_loop,
            &popup,
            "({opener:window.opener!==null,origin:window.opener.location.origin})"
        ),
        serde_json::json!({"opener":true,"origin":server.origin()})
    );
    assert_eq!(
        evaluate(&mut event_loop, &parent, "window.messages"),
        serde_json::json!(["popup-ready"])
    );
    popup.evaluate_script("window.close()").unwrap();
    wait_for(
        &mut event_loop,
        "popup close request",
        |event| match event {
            TestEvent::Request(RequestEvent::CloseRequested { tab_id }) if tab_id == POPUP_TAB => {
                Some(())
            }
            _ => None,
        },
    );
    drop(popup_attachment);
    drop(popup);

    parent
        .evaluate_script("window.blankPopup=window.open('about:blank','blank-request-test')")
        .unwrap();
    let blank_request = wait_for_request(&mut event_loop, SitePermission::Popups);
    assert_eq!(blank_request.target_url.as_deref(), Some("about:blank"));
    let (blank, blank_attachment) = make_view(
        &event_loop,
        &window,
        &mut context,
        &broker,
        BLANK_POPUP_TAB,
        Some(broker.popup_environment(blank_request.id).unwrap()),
    );
    broker.resolve_popup(blank_request.id, &blank).unwrap();
    wait_for_load(&mut event_loop, BLANK_POPUP_TAB);
    assert_eq!(
        evaluate(&mut event_loop, &blank, "window.opener.location.origin"),
        serde_json::json!(server.origin())
    );
    drop(blank_attachment);
    drop(blank);

    // An iframe's popup belongs to its own origin, never the embedding page's saved grant.
    let embedded_server = FixtureServer::new();
    parent.evaluate_script(&format!(
        "window.requestFrame=document.createElement('iframe');requestFrame.src={};document.body.append(requestFrame)",
        serde_json::json!(embedded_server.url("/embedded-popup"))
    )).unwrap();
    let embedded_popup = wait_for_request(&mut event_loop, SitePermission::Popups);
    assert_eq!(embedded_popup.origin, embedded_server.origin());
    assert_ne!(embedded_popup.origin, server.origin());
    broker.deny(embedded_popup.id).unwrap();
    assert!(broker.pending_requests().is_empty());
    parent.evaluate_script("requestFrame.remove()").unwrap();

    request_location(&parent);
    let permission = wait_for_request(&mut event_loop, SitePermission::Location);
    assert_eq!(permission.origin, server.origin());
    broker.resolve_permission(permission.id, false).unwrap();
    wait_for(
        &mut event_loop,
        "denied geolocation result",
        |event| match event {
            TestEvent::Title(id, title) if id == PARENT_TAB && title == "denied-1" => Some(()),
            _ => None,
        },
    );
    assert_eq!(
        permission_count(&mut event_loop, &parent),
        0,
        "decisions must not persist inside WebView2"
    );

    request_location(&parent);
    let stale = wait_for_request(&mut event_loop, SitePermission::Location);
    parent.load_url(&server.url("/again")).unwrap();
    wait_for(
        &mut event_loop,
        "navigation cancellation",
        |event| match event {
            TestEvent::Request(RequestEvent::Cancelled(ids)) if ids.contains(&stale.id) => Some(()),
            _ => None,
        },
    );
    assert!(broker.pending(stale.id).is_none());
    assert!(broker.resolve_permission(stale.id, true).is_err());
    wait_for_load(&mut event_loop, PARENT_TAB);

    request_location(&parent);
    let closing = wait_for_request(&mut event_loop, SitePermission::Location);
    drop(parent_attachment);
    assert!(broker.pending(closing.id).is_none());
    assert!(broker.resolve_permission(closing.id, true).is_err());
    drop(parent);
    drop(broker);
    drop(context);
    drop(window);
    drop(event_loop);
    profile.purge_ephemeral_storage().unwrap();
}
