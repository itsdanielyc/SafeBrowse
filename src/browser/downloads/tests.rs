use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

use crate::ui::kiosk::KioskEvent;

const NATIVE_TIMEOUT: Duration = Duration::from_secs(15);
const FIXTURE_BODY: &[u8] = b"SafeBrowse disposable download fixture. No executable content.\n";

#[test]
fn failed_request_cancellation_uses_the_native_operation_fallback() {
    let fallback_calls = std::cell::Cell::new(0);
    let result = establish_initial_denial(
        || Err("SetCancel failed".into()),
        || {
            fallback_calls.set(fallback_calls.get() + 1);
            Ok(())
        },
    );
    assert_eq!(fallback_calls.get(), 1);
    assert!(
        matches!(result, Err(InitialDenialFailure::Cancelled(message)) if message.contains("SetCancel failed"))
    );

    let result = establish_initial_denial(
        || Ok(()),
        || panic!("successful primary denial must not cancel a potentially approvable operation"),
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn failure_of_both_cancellation_paths_requires_tab_teardown() {
    let fallback_calls = std::cell::Cell::new(0);
    let result = establish_initial_denial(
        || Err("SetCancel failed".into()),
        || {
            fallback_calls.set(fallback_calls.get() + 1);
            Err("DownloadOperation.Cancel failed".into())
        },
    );
    assert_eq!(fallback_calls.get(), 1);
    assert!(
        matches!(result, Err(InitialDenialFailure::Unprotected(message)) if message.contains("SetCancel failed") && message.contains("DownloadOperation.Cancel failed"))
    );
}

#[test]
fn windows_download_names_cannot_escape_or_alias_reserved_paths() {
    for (name, expected) in [
        ("../../invoice.pdf", "invoice.pdf"),
        ("C:\\elsewhere\\invoice.pdf", "invoice.pdf"),
        ("document.pdf:payload.exe", "document.pdf_payload.exe"),
        ("NUL.txt", "_NUL.txt"),
        ("COM1", "_COM1"),
        ("LPT².txt", "_LPT².txt"),
        ("...", "download.bin"),
        ("report\u{202e}fdp.exe", "report_fdp.exe"),
    ] {
        assert_eq!(safe_file_name(name), expected);
    }
    let long = format!("{}.pdf", "\u{1f512}".repeat(200));
    let safe = safe_file_name(&long);
    assert!(safe.encode_utf16().count() <= 120);
    assert!(safe.ends_with(".pdf"));
}

#[test]
fn downloads_reject_external_schemes_and_foreign_blob_origins() {
    for uri in [
        "https://cdn.example/file",
        "http://example.com/file",
        "blob:https://example.com/id",
    ] {
        assert!(validate_download_url(uri, "https://example.com").is_ok());
    }
    for uri in [
        "file:///C:/Windows/win.ini",
        "data:text/plain,hello",
        "blob:https://elsewhere.example/id",
        "javascript:alert(1)",
        "blob:null/id",
    ] {
        assert!(validate_download_url(uri, "https://example.com").is_err());
    }
}

#[test]
fn repeated_download_names_receive_distinct_nonexisting_paths() {
    let fixture = FixtureDirectory::new();
    let first = DownloadDestination::new(Some(&fixture.path), "receipt.txt").unwrap();
    std::fs::write(first.path(), FIXTURE_BODY).unwrap();
    let second = DownloadDestination::new(Some(&fixture.path), "receipt.txt").unwrap();
    assert_ne!(first.path(), second.path());
    assert!(!second.path().exists());
    assert_eq!(std::fs::read(first.path()).unwrap(), FIXTURE_BODY);
}

/// Uses hidden native views, a loopback-only server and a disposable output directory.
#[test]
fn native_downloads_wait_for_approval_and_cancel_with_their_owner() {
    let _lock = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
    let fixture = FixtureDirectory::new();
    let server = FixtureServer::new();
    let mut builder = EventLoopBuilder::<KioskEvent>::with_user_event();
    builder.with_any_thread(true);
    let mut event_loop = builder.build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .unwrap();
    let mut context = WebContext::new(Some(fixture.path.join("profile")));
    let view = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        .with_devtools(false)
        .with_browser_accelerator_keys(false)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .build_as_child(&window)
        .unwrap();
    crate::browser::security::harden_content_view(&view).unwrap();
    let mut broker = DownloadBroker::new();
    broker.destination_root = Some(fixture.path.join("output"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = Rc::clone(&events);
    let attachment = broker
        .attach(&view, 1, move |event| received.borrow_mut().push(event))
        .unwrap();
    load_fixture_page(&mut event_loop, &view, &server);

    let notice = request_download(&mut event_loop, &view, &server, &broker);
    assert_eq!(notice.origin, server.origin());
    assert_eq!(notice.total_bytes, Some(FIXTURE_BODY.len() as u64));
    assert!(
        !fixture.path.join("output").exists(),
        "unapproved download allocated an output path"
    );
    pump_until(&mut event_loop, || !broker.pending_requests().is_empty());
    broker.resolve(notice.id, false).unwrap();
    assert!(broker.pending_requests().is_empty());
    assert!(!fixture.path.join("output").exists());
    assert!(broker.resolve(notice.id, true).is_err());

    load_fixture_page(&mut event_loop, &view, &server);
    let notice = request_download(&mut event_loop, &view, &server, &broker);
    broker.clone().resolve(notice.id, true).unwrap();
    pump_until(&mut event_loop, || {
        events.borrow().iter().any(|event| matches!(event, DownloadEvent::Completed { notice: completed, .. } if completed.id == notice.id))
    });
    let path = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            DownloadEvent::Completed {
                notice: completed,
                path,
            } if completed.id == notice.id => Some(PathBuf::from(path)),
            _ => None,
        })
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), FIXTURE_BODY);
    assert!(broker.registry.borrow().active.is_empty());
    assert!(path.starts_with(fixture.path.join("output")));

    load_fixture_page(&mut event_loop, &view, &server);
    view.load_url(&format!("{}/incomplete", server.origin()))
        .unwrap();
    pump_until(&mut event_loop, || !broker.pending_requests().is_empty());
    let interrupted = broker.pending_requests().into_iter().next().unwrap();
    let interrupted_operation = broker
        .registry
        .borrow()
        .pending
        .get(&interrupted.id)
        .unwrap()
        .operation
        .clone();
    events.borrow_mut().clear();
    broker.resolve(interrupted.id, true).unwrap();
    // Force a native terminal interruption without waiting for Chromium's network retry backoff.
    unsafe {
        interrupted_operation.Cancel().unwrap();
    }
    pump_until(&mut event_loop, || {
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, DownloadEvent::Failed { .. }))
    });
    assert!(
        broker.registry.borrow().active.is_empty(),
        "interrupted transfer retained an active slot"
    );
    assert!(!events
        .borrow()
        .iter()
        .any(|event| matches!(event, DownloadEvent::Completed { .. })));

    // Keep four distinct tabs' transfers pending at the server to exercise the real active cap.
    let mut slow_views = Vec::new();
    for index in 0..=MAX_ACTIVE_DOWNLOADS {
        let slow_view = WebViewBuilder::new_with_web_context(&mut context)
            .with_visible(false)
            .with_permission_handler(|_| wry::PermissionResponse::Deny)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .build_as_child(&window)
            .unwrap();
        crate::browser::security::harden_content_view(&slow_view).unwrap();
        let attachment = broker.attach(&slow_view, index + 2, |_| {}).unwrap();
        load_fixture_page(&mut event_loop, &slow_view, &server);
        slow_view
            .load_url(&format!("{}/slow?{index}", server.origin()))
            .unwrap();
        pump_until(&mut event_loop, || !broker.pending_requests().is_empty());
        let waiting = broker.pending_requests().into_iter().next().unwrap();
        if index < MAX_ACTIVE_DOWNLOADS {
            broker.resolve(waiting.id, true).unwrap();
            assert_eq!(broker.registry.borrow().active.len(), index + 1);
        } else {
            let directory_count = std::fs::read_dir(fixture.path.join("output"))
                .unwrap()
                .count();
            assert!(broker
                .resolve(waiting.id, true)
                .unwrap_err()
                .contains("Too many active downloads"));
            assert_eq!(
                std::fs::read_dir(fixture.path.join("output"))
                    .unwrap()
                    .count(),
                directory_count,
                "rejected transfer allocated a destination"
            );
            assert!(broker.pending_requests().is_empty());
        }
        slow_views.push((attachment, slow_view));
    }
    broker.cancel_all();
    assert!(
        broker.registry.borrow().active.is_empty(),
        "cancel_all retained active transfers"
    );
    drop(slow_views);

    load_fixture_page(&mut event_loop, &view, &server);
    let pending = request_download(&mut event_loop, &view, &server, &broker);
    load_fixture_page(&mut event_loop, &view, &server);
    assert!(
        broker.pending(pending.id).is_none(),
        "navigation retained approval for a previous document"
    );

    let pending = request_download(&mut event_loop, &view, &server, &broker);
    drop(attachment);
    assert!(
        broker.pending(pending.id).is_none(),
        "tab attachment retained its pending download"
    );

    let received = Rc::clone(&events);
    let attachment = broker
        .attach(&view, 1, move |event| received.borrow_mut().push(event))
        .unwrap();
    load_fixture_page(&mut event_loop, &view, &server);
    let pending = request_download(&mut event_loop, &view, &server, &broker);
    let native_arguments = broker
        .registry
        .borrow()
        .pending
        .get(&pending.id)
        .unwrap()
        .arguments
        .clone();
    drop(broker);
    let mut cancelled = webview2_core::BOOL(0);
    unsafe {
        native_arguments.Cancel(&mut cancelled).unwrap();
    }
    assert!(
        cancelled.as_bool(),
        "dropping the final broker did not fail closed"
    );
    drop(attachment);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        FIXTURE_BODY,
        "cancellation changed a completed file"
    );
    drop(view);
    drop(context);
    drop(window);
}

fn load_fixture_page(
    event_loop: &mut EventLoop<KioskEvent>,
    view: &WebView,
    server: &FixtureServer,
) {
    let expected_url = format!("{}/page?{}", server.origin(), uuid::Uuid::new_v4());
    view.load_url(&expected_url).unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let evaluating = Arc::new(AtomicBool::new(false));
    let script = format!(
        "location.href === {} && document.readyState === 'complete'",
        serde_json::json!(expected_url)
    );
    pump_until(event_loop, || {
        if finished.load(Ordering::Relaxed) {
            return true;
        }
        if !evaluating.swap(true, Ordering::Relaxed) {
            let response = Arc::clone(&finished);
            let pending = Arc::clone(&evaluating);
            view.evaluate_script_with_callback(&script, move |state| {
                response.store(state == "true", Ordering::Relaxed);
                pending.store(false, Ordering::Relaxed);
            })
            .unwrap();
        }
        false
    });
}

fn request_download(
    event_loop: &mut EventLoop<KioskEvent>,
    view: &WebView,
    server: &FixtureServer,
    broker: &DownloadBroker,
) -> DownloadNotice {
    view.load_url(&format!(
        "{}/payload?{}",
        server.origin(),
        uuid::Uuid::new_v4()
    ))
    .unwrap();
    pump_until(event_loop, || !broker.pending_requests().is_empty());
    broker.pending_requests().into_iter().next().unwrap()
}

fn pump_until(event_loop: &mut EventLoop<KioskEvent>, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + NATIVE_TIMEOUT;
    event_loop.run_return(|_, _, flow| {
        *flow = ControlFlow::WaitUntil((Instant::now() + Duration::from_millis(10)).min(deadline));
        if ready() || Instant::now() >= deadline {
            *flow = ControlFlow::Exit;
        }
    });
    assert!(ready(), "native download fixture timed out");
}

struct FixtureDirectory {
    path: PathBuf,
}

impl FixtureDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("safebrowse-download-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self { path }
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let expected_parent = std::env::temp_dir();
        if self.path.parent() == Some(expected_parent.as_path())
            && self.path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("safebrowse-download-test-")
            })
        {
            for _ in 0..10 {
                if std::fs::remove_dir_all(&self.path).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct FixtureServer {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let worker = std::thread::spawn(move || {
            let mut connections = Vec::new();
            for stream in listener.incoming() {
                if worker_stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let stopped = Arc::clone(&worker_stopped);
                connections.push(std::thread::spawn(move || {
                    serve_connection(stream, &stopped)
                }));
            }
            for connection in connections {
                let _ = connection.join();
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
}

fn serve_connection(mut stream: TcpStream, stopped: &AtomicBool) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut first_line = String::new();
    if BufReader::new(&stream).read_line(&mut first_line).is_err() {
        return;
    }
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let is_slow = path.starts_with("/slow");
    let incomplete = path.starts_with("/incomplete");
    let (headers, body) = if path.starts_with("/payload") || is_slow || incomplete {
        ("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"fixture.txt\"\r\n", FIXTURE_BODY)
    } else {
        (
            "Content-Type: text/html\r\n",
            b"<!doctype html><title>Download fixture</title><p>Disposable local test</p>"
                .as_slice(),
        )
    };
    let declared_length = if is_slow || incomplete {
        body.len() * 2
    } else {
        body.len()
    };
    let response = format!("HTTP/1.1 200 OK\r\n{headers}Content-Length: {declared_length}\r\nConnection: close\r\n\r\n");
    let _ = stream
        .write_all(response.as_bytes())
        .and_then(|_| stream.write_all(body));
    let deadline = Instant::now() + NATIVE_TIMEOUT;
    while is_slow && !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
