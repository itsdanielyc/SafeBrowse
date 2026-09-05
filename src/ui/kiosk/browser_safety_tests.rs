/// Loopback-only HTML used by the production-builder regressions; it contains no native print call.
struct BrowserSafetyServer {
    address: std::net::SocketAddr,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    document_requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl BrowserSafetyServer {
    fn new() -> Self {
        use std::io::{BufRead, Write};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
        const ROOT_DOCUMENT: &str = r#"<!doctype html><title>Browser safety fixture</title><p id="fixture-marker">disposable-browser-safety</p><iframe id="fixture-frame" src="/frame"></iframe>"#;
        const FRAME_DOCUMENT: &str =
            "<!doctype html><title>Child fixture</title><p>Dummy frame</p>";

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let document_requests = Arc::new(AtomicUsize::new(0));
        let worker_stopped = Arc::clone(&stopped);
        let worker_requests = Arc::clone(&document_requests);
        let worker = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if worker_stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = connection else { continue };
                let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
                let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
                let mut first_line = String::new();
                if std::io::BufReader::new(&stream)
                    .read_line(&mut first_line)
                    .is_err()
                {
                    continue;
                }
                let path = first_line.split_whitespace().nth(1).unwrap_or_default();
                let body = match path {
                    "/top" | "/popup" | "/crash" => {
                        worker_requests.fetch_add(1, Ordering::Relaxed);
                        ROOT_DOCUMENT
                    }
                    "/frame" => FRAME_DOCUMENT,
                    _ => "",
                };
                let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream
                    .write_all(headers.as_bytes())
                    .and_then(|_| stream.write_all(body.as_bytes()));
            }
        });
        Self {
            address,
            stopped,
            document_requests,
            worker: Some(worker),
        }
    }

    fn url(&self, route: &str) -> String {
        format!("http://{}{route}", self.address)
    }

    fn document_count(&self) -> usize {
        self.document_requests
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for BrowserSafetyServer {
    fn drop(&mut self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn evaluate_website_fixture(
    event_loop: &mut EventLoop<KioskEvent>,
    view: &WebView,
    script: &str,
) -> Value {
    let (sender, receiver) = std::sync::mpsc::channel();
    let proxy = event_loop.create_proxy();
    view.evaluate_script_with_callback(script, move |value| {
        let _ = sender.send(value);
        let _ = proxy.send_event(KioskEvent::Notice("Browser safety fixture evaluated"));
    })
    .unwrap();
    let result = wait_for_native_result(event_loop, |_| receiver.try_recv().ok());
    serde_json::from_str(&result).unwrap()
}

fn load_website_fixture(
    event_loop: &mut EventLoop<KioskEvent>,
    view: &WebView,
    tab_id: usize,
    url: &str,
) {
    view.load_url(url).unwrap();
    wait_for_native_result(event_loop, |event| {
        matches!(event, Event::UserEvent(KioskEvent::PageLoad { id, loading: false }) if id == tab_id)
            .then_some(())
    });
}

/// Refuses to invoke print until the real production wrapper has been positively identified.
fn assert_website_print_guard(
    event_loop: &mut EventLoop<KioskEvent>,
    view: &WebView,
    has_frame: bool,
) {
    let observations = evaluate_website_fixture(
        event_loop,
        view,
        &format!(
            r#"
        (() => {{
            function inspect(target) {{
                const descriptor = Object.getOwnPropertyDescriptor(target, 'print');
                const source = Function.prototype.toString.call(target.print);
                const guarded = descriptor?.configurable === false
                    && typeof descriptor.get === 'function'
                    && typeof descriptor.set === 'function'
                    && source.includes('renderingNotice')
                    && !source.includes('[native code]');
                if (!guarded) return {{ guarded: false }};
                target.print();
                target.print();
                return {{ guarded: true, notices: target.document.querySelectorAll('[aria-label="Website printing"]').length }};
            }}
            return {{ top: inspect(window), frame: {has_frame} ? inspect(document.getElementById('fixture-frame').contentWindow) : null }};
        }})()
    "#
        ),
    );
    assert_eq!(observations["top"]["guarded"], true, "{observations}");
    assert_eq!(observations["top"]["notices"], 1);
    if has_frame {
        assert_eq!(observations["frame"]["guarded"], true, "{observations}");
        assert_eq!(observations["frame"]["notices"], 1);
    }
}

#[test]
fn production_website_and_popup_builders_install_print_guard_including_frames() {
    let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
    let server = BrowserSafetyServer::new();
    let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
    let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .unwrap();
    let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
    let requests = RequestBroker::new();
    let downloads = DownloadBroker::new();
    let website = build_content_view(
        &window,
        &mut context,
        1,
        &event_loop.create_proxy(),
        &requests,
        &downloads,
        None,
    )
    .unwrap();
    load_website_fixture(&mut event_loop, &website, 1, &server.url("/top"));
    assert_website_print_guard(&mut event_loop, &website, true);

    let popup = build_content_view(
        &window,
        &mut context,
        2,
        &event_loop.create_proxy(),
        &requests,
        &downloads,
        Some(website.environment()),
    )
    .unwrap();
    load_website_fixture(&mut event_loop, &popup, 2, "about:blank");
    assert_website_print_guard(&mut event_loop, &popup, false);
    load_website_fixture(&mut event_loop, &popup, 2, &server.url("/popup"));
    assert_website_print_guard(&mut event_loop, &popup, true);
    drop(popup);
    drop(website);
    drop(context);
    profile.purge_ephemeral_storage().unwrap();
}

struct FixtureBrowserProcess(windows::Win32::Foundation::HANDLE);

impl Drop for FixtureBrowserProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Terminates one retained handle only after a unique disposable profile and live page are verified.
#[test]
fn production_engine_monitor_reports_disposable_browser_process_exit_without_reload() {
    use crate::browser::health::BrowserFailure;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    const FIXTURE_TAB_ID: usize = 31;
    const PROCESS_EXIT_TIMEOUT_MS: u32 = 5000;
    const FIXTURE_EXIT_CODE: u32 = 90;
    let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
    let server = BrowserSafetyServer::new();
    let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
    let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .build(&event_loop)
        .unwrap();
    let mut context = WebContext::new(Some(profile.data_directory().to_owned()));
    let requests = RequestBroker::new();
    let downloads = DownloadBroker::new();
    let website = build_content_view(
        &window,
        &mut context,
        FIXTURE_TAB_ID,
        &event_loop.create_proxy(),
        &requests,
        &downloads,
        None,
    )
    .unwrap();
    crate::browser::runtime::validate_created_environment(&website, profile.data_directory())
        .unwrap();
    load_website_fixture(
        &mut event_loop,
        &website,
        FIXTURE_TAB_ID,
        &server.url("/crash"),
    );
    let core = unsafe { website.controller().CoreWebView2().unwrap() };
    let mut browser_pid = 0;
    unsafe {
        core.BrowserProcessId(&mut browser_pid).unwrap();
    }
    assert_ne!(browser_pid, 0);
    assert_ne!(browser_pid, std::process::id());
    let process = FixtureBrowserProcess(unsafe {
        OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, browser_pid).unwrap()
    });
    // A page evaluation after handle acquisition proves that the engine is still the live owner.
    // If it exited before OpenProcess and the PID was reused, this check fails before termination.
    let marker = evaluate_website_fixture(
        &mut event_loop,
        &website,
        "document.getElementById('fixture-marker')?.textContent",
    );
    assert_eq!(marker, "disposable-browser-safety");
    let mut current_browser_pid = 0;
    unsafe {
        core.BrowserProcessId(&mut current_browser_pid).unwrap();
    }
    assert_eq!(browser_pid, current_browser_pid);
    assert_eq!(server.document_count(), 1);
    unsafe {
        TerminateProcess(process.0, FIXTURE_EXIT_CODE).unwrap();
    }
    let failure = wait_for_native_result(&mut event_loop, |event| match event {
        Event::UserEvent(KioskEvent::EngineHealth {
            tab_id: Some(FIXTURE_TAB_ID),
            event: BrowserHealthEvent::Failed(failure),
        }) => Some(failure),
        _ => None,
    });
    assert_eq!(failure, BrowserFailure::BrowserExited);
    assert_eq!(
        unsafe { WaitForSingleObject(process.0, PROCESS_EXIT_TIMEOUT_MS) },
        WAIT_OBJECT_0
    );
    assert_eq!(
        server.document_count(),
        1,
        "the failure callback must not replay the page request"
    );
    drop(core);
    drop(website);
    drop(context);
    profile.purge_ephemeral_storage().unwrap();
}

/// Pumps native teardown callbacks until the engine exits and its queued notifications settle.
fn assert_normal_fixture_shutdown(
    event_loop: &mut EventLoop<KioskEvent>,
    process: &FixtureBrowserProcess,
) {
    use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};

    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
    const PROCESS_CHECK_INTERVAL: Duration = Duration::from_millis(20);
    const EXIT_NOTIFICATION_GRACE: Duration = Duration::from_millis(200);
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    let mut exited_at = None;
    let mut unexpected_failure = None;
    let mut process_wait_error = None;
    event_loop.run_return(|event, _, control_flow| {
        let now = Instant::now();
        *control_flow = ControlFlow::WaitUntil((now + PROCESS_CHECK_INTERVAL).min(deadline));
        if let Event::UserEvent(KioskEvent::EngineHealth {
            event: BrowserHealthEvent::Failed(failure),
            ..
        }) = event
        {
            unexpected_failure = Some(failure);
        }
        match unsafe { WaitForSingleObject(process.0, 0) } {
            WAIT_OBJECT_0 => {
                exited_at.get_or_insert(now);
            }
            WAIT_TIMEOUT => {}
            result => process_wait_error = Some(result),
        }
        let notifications_settled = exited_at
            .is_some_and(|exit_time| now.duration_since(exit_time) >= EXIT_NOTIFICATION_GRACE);
        if unexpected_failure.is_some()
            || process_wait_error.is_some()
            || notifications_settled
            || now >= deadline
        {
            *control_flow = ControlFlow::Exit;
        }
    });
    assert_eq!(process_wait_error, None, "cannot observe fixture engine exit");
    assert_eq!(
        unexpected_failure, None,
        "ordinary view teardown must not report an engine failure"
    );
    assert!(exited_at.is_some(), "fixture engine did not exit normally");
    let mut exit_code = 0;
    unsafe { GetExitCodeProcess(process.0, &mut exit_code).unwrap() };
    assert_eq!(exit_code, 0, "fixture engine exited abnormally");
}

/// Repeated ordinary shutdown must remove real runtime data without turning teardown into failure.
#[test]
fn production_normal_shutdown_removes_profiles_without_reporting_engine_failure() {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    const SESSION_COUNT: usize = 3;
    const FIXTURE_TAB_ID: usize = 51;
    let _native_test = crate::ui::NATIVE_WEBVIEW_TEST_LOCK.lock().unwrap();
    let server = BrowserSafetyServer::new();
    for session_index in 0..SESSION_COUNT {
        let profile = ProfileManager::new(ProfileMode::Ephemeral).unwrap();
        let profile_directory = profile.data_directory().to_owned();
        let mut event_loop = EventLoopBuilder::<KioskEvent>::with_user_event()
            .with_any_thread(true)
            .build();
        let window = WindowBuilder::new()
            .with_visible(false)
            .build(&event_loop)
            .unwrap();
        let mut context = WebContext::new(Some(profile_directory.clone()));
        let requests = RequestBroker::new();
        let downloads = DownloadBroker::new();
        let website = build_content_view(
            &window,
            &mut context,
            FIXTURE_TAB_ID,
            &event_loop.create_proxy(),
            &requests,
            &downloads,
            None,
        )
        .unwrap();
        load_website_fixture(
            &mut event_loop,
            &website,
            FIXTURE_TAB_ID,
            &server.url("/top"),
        );
        let marker = evaluate_website_fixture(
            &mut event_loop,
            &website,
            "localStorage.setItem('shutdown-fixture', 'disposable-browser-safety'); document.getElementById('fixture-marker')?.textContent",
        );
        assert_eq!(marker, "disposable-browser-safety");
        let process = {
            let core = unsafe { website.controller().CoreWebView2().unwrap() };
            let mut browser_pid = 0;
            unsafe { core.BrowserProcessId(&mut browser_pid).unwrap() };
            assert_ne!(browser_pid, 0);
            assert_ne!(browser_pid, std::process::id());
            FixtureBrowserProcess(unsafe {
                OpenProcess(
                    PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    browser_pid,
                )
                .unwrap()
            })
        };
        // Match production's release-before-purge order while retaining no WebView2 COM references.
        drop(website);
        drop(context);
        let cleanup_result = profile.purge_ephemeral_storage();
        assert_normal_fixture_shutdown(&mut event_loop, &process);
        cleanup_result.unwrap_or_else(|error| panic!("session {session_index}: {error}"));
        assert!(
            !profile_directory.try_exists().unwrap(),
            "session {session_index} left its temporary browser profile behind"
        );
    }
    assert_eq!(server.document_count(), SESSION_COUNT);
}
