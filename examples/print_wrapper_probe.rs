//! Inspects print bindings in hidden WebViews; invokes only the verified local guidance wrapper.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use safebrowse::browser::requests::{RequestAttachment, RequestBroker, RequestEvent};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment;
use wry::{PageLoadEvent, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

const TIMEOUT: Duration = Duration::from_secs(15);
const WRAPPER: &str = include_str!("../src/browser/printing/website_print_guard.js");
const SNAPSHOT: &str = r#"
function snapshot(w) {
    try {
        const descriptor = Object.getOwnPropertyDescriptor(w, 'print');
        const text = Function.prototype.toString.call(w.print);
        return { exists:typeof w.print === 'function', own:Boolean(descriptor), writable:descriptor?.writable,
            configurable:descriptor?.configurable, accessor:typeof descriptor?.get === 'function', wrapped:text.includes('showSuppressionNotice'), native:text.includes('[native code]'), text };
    } catch (error) { return { error:String(error) }; }
}
"#;

#[derive(Debug)]
enum ProbeEvent {
    Loaded(usize, String),
    Request(RequestEvent),
    Evaluated(String),
}

fn main() {
    let fixture = FixtureDirectory::new();
    let secondary = FixtureServer::new(None);
    let server = FixtureServer::new(Some(secondary.origin()));
    let mut event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_visible(false)
        .with_focused(false)
        .build(&event_loop)
        .unwrap();
    let mut context = WebContext::new(Some(fixture.path.join("profile")));
    let broker = RequestBroker::new();
    let (parent, parent_attachment) =
        make_view(&event_loop, &window, &mut context, &broker, 1, None);
    parent.load_url(&server.url("/parent")).unwrap();
    wait_for(&mut event_loop, "parent load", |event| match event {
        ProbeEvent::Loaded(1, url) if url.contains("/parent") => Some(()),
        _ => None,
    });
    let mut popup_views = Vec::new();
    for (id, name, destination) in [
        (2, "httpPopup", server.url("/popup")),
        (3, "blankPopup", "about:blank".into()),
    ] {
        parent.evaluate_script(&format!(
            "window.{name}=window.open({}, '{}'); results.{name}Immediate=snapshot(window.{name}); references.{name}=window.{name}?.print;",
            serde_json::json!(destination), name
        )).unwrap();
        let request = wait_for(&mut event_loop, "popup request", |event| match event {
            ProbeEvent::Request(RequestEvent::Requested(request)) => Some(request),
            _ => None,
        });
        let (popup, attachment) = make_view(
            &event_loop,
            &window,
            &mut context,
            &broker,
            id,
            Some(broker.popup_environment(request.id).unwrap()),
        );
        broker.resolve_popup(request.id, &popup).unwrap();
        wait_for(&mut event_loop, "popup load", |event| match event {
            ProbeEvent::Loaded(loaded_id, url) if loaded_id == id && url == destination => Some(()),
            _ => None,
        });
        parent.evaluate_script(&format!(
            "results.{name}Later=snapshot(window.{name}); results.{name}Retained={{native:Function.prototype.toString.call(references.{name}).includes('[native code]'),same:references.{name}===window.{name}.print}};"
        )).unwrap();
        popup_views.push((attachment, popup));
    }
    let report = evaluate(&mut event_loop, &parent, "results");
    let guidance = evaluate(
        &mut event_loop,
        &parent,
        r#"
    (() => {
        const descriptor=Object.getOwnPropertyDescriptor(window,'print');
        const guarded=window.print;
        const source=Function.prototype.toString.call(guarded);
        if (descriptor?.configurable!==false || typeof descriptor.get!=='function' || !source.startsWith('function showSuppressionNotice(') || source.includes('[native code]')) return {invoked:false};
        guarded();guarded();
        const notices=document.querySelectorAll('[role="status"][aria-label="Website printing"]');
        return {invoked:true,count:notices.length,text:notices[0]?.textContent};
    })()
    "#,
    );
    let report = serde_json::json!({"runtime":wry::webview_version().unwrap(),"native_print_invoked":false,"results":report,"guarded_guidance":guidance});
    let report_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/website_print_guard_probe_report.json");
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!(
        "Runtime: {}; native print invoked: false; guarded guidance: {}",
        report["runtime"], report["guarded_guidance"]
    );
    for (name, value) in report["results"].as_object().unwrap() {
        let mut summary = value.clone();
        if let Some(object) = summary.as_object_mut() {
            object.remove("text");
        }
        if name != "frames" {
            println!("{name}: {summary}");
        }
    }
    println!("Report: {}", report_path.display());
    drop(popup_views);
    drop(parent_attachment);
    drop(parent);
    drop(broker);
    drop(context);
    drop(window);
}

fn make_view(
    event_loop: &EventLoop<ProbeEvent>,
    window: &Window,
    context: &mut WebContext,
    broker: &RequestBroker,
    id: usize,
    environment: Option<ICoreWebView2Environment>,
) -> (WebView, RequestAttachment) {
    let loaded = event_loop.create_proxy();
    let mut builder = WebViewBuilder::new_with_web_context(context)
        .with_visible(false)
        .with_devtools(false)
        .with_initialization_script(WRAPPER)
        .with_download_started_handler(|_, _| false)
        .with_navigation_handler(|uri| {
            uri.starts_with("http://127.0.0.1:") || uri == "about:blank" || uri == "about:srcdoc"
        })
        .with_on_page_load_handler(move |kind, url| {
            if matches!(kind, PageLoadEvent::Finished) {
                let _ = loaded.send_event(ProbeEvent::Loaded(id, url));
            }
        });
    if let Some(environment) = environment {
        builder = builder.with_environment(environment);
    }
    let view = builder.build_as_child(window).unwrap();
    let request_proxy = event_loop.create_proxy();
    let attachment = broker
        .attach(&view, id, move |event| {
            let _ = request_proxy.send_event(ProbeEvent::Request(event));
        })
        .unwrap();
    (view, attachment)
}

fn wait_for<T>(
    event_loop: &mut EventLoop<ProbeEvent>,
    description: &str,
    mut observe: impl FnMut(ProbeEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + TIMEOUT;
    let mut found = None;
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::WaitUntil(deadline);
        if let Event::UserEvent(event) = event {
            if let ProbeEvent::Request(RequestEvent::Failed { message, .. }) = &event {
                panic!("{message}");
            }
            if found.is_none() {
                found = observe(event);
            }
        }
        if found.is_some() || Instant::now() >= deadline {
            *flow = ControlFlow::Exit;
        }
    });
    found.unwrap_or_else(|| panic!("Timed out: {description}"))
}

fn evaluate(
    event_loop: &mut EventLoop<ProbeEvent>,
    view: &WebView,
    script: &str,
) -> serde_json::Value {
    let proxy = event_loop.create_proxy();
    view.evaluate_script_with_callback(script, move |value| {
        let _ = proxy.send_event(ProbeEvent::Evaluated(value));
    })
    .unwrap();
    let value = wait_for(event_loop, "inspection response", |event| match event {
        ProbeEvent::Evaluated(value) => Some(value),
        _ => None,
    });
    serde_json::from_str(&value).unwrap()
}

fn parent_html(cross_origin: &str) -> String {
    let scripts = format!(
        r#"
<script>{SNAPSHOT}
window.results = {{inlineTop:snapshot(window),frames:{{}}}};window.references={{}};
window.addEventListener('message', event => {{if(event.data?.probe) results.frames[event.data.id]=event.data.snapshot}});
const original = window.print;
try {{window.print=function replacement(){{}};}} catch(error) {{results.assignmentError=String(error)}}
results.assignmentPreserved=window.print===original;
results.deleteResult=delete window.print;
try {{Object.defineProperty(window,'print',{{value:function replacement(){{}}}});results.redefineSucceeded=true}} catch(error) {{results.redefineSucceeded=false}}
results.afterMutation=snapshot(window);
</script>
<iframe id="regular" src="/child?regular"></iframe>
<iframe id="cross" src="{cross_origin}/child?cross"></iframe>
<iframe id="staticBlank" src="about:blank"></iframe>
<iframe id="staticSrcdoc" srcdoc="&lt;script&gt;parent.postMessage({{probe:true,id:'staticSrcdocInline',snapshot:parent.snapshot(window)}},'*')&lt;/script&gt;"></iframe>
<script>
for(const name of ['staticBlank','staticSrcdoc']) {{
 const frame=document.getElementById(name);results[name+'Early']=snapshot(frame.contentWindow);references[name]=frame.contentWindow.print;
}}
function makeFrame(name,srcdoc,write) {{
 const frame=document.createElement('iframe');frame.id=name;
 if(srcdoc) frame.srcdoc="<script>parent.postMessage({{probe:true,id:'dynamicSrcdocInline',snapshot:parent.snapshot(window)}},'*')<"+"/script>";
 document.body.append(frame);
 results[name+'Early']=snapshot(frame.contentWindow);references[name]=frame.contentWindow.print;
 if(write) {{frame.contentWindow.document.open();frame.contentWindow.document.write("<script>parent.postMessage({{probe:true,id:'documentWriteInline',snapshot:parent.snapshot(window)}},'*')<"+"/script>");frame.contentWindow.document.close();}}
}}
makeFrame('dynamicBlank',false,false);makeFrame('dynamicSrcdoc',true,false);makeFrame('documentWrite',false,true);
window.addEventListener('load',()=>{{
 results.loadedTop=snapshot(window);
 for(const name of ['regular','staticBlank','staticSrcdoc','dynamicBlank','dynamicSrcdoc','documentWrite']) {{
   const frame=document.getElementById(name);results[name+'Later']=snapshot(frame.contentWindow);
   if(references[name]) results[name+'Retained']={{native:Function.prototype.toString.call(references[name]).includes('[native code]'),same:references[name]===frame.contentWindow.print}};
 }}
}});
</script>
"#
    );
    format!("<!doctype html><meta charset='utf-8'><title>Print wrapper inspection</title><body>{scripts}</body>")
}

fn child_html(id: &str) -> String {
    format!("<!doctype html><meta charset='utf-8'><title>Frame probe</title><script>{SNAPSHOT}parent.postMessage({{probe:true,id:{},snapshot:snapshot(window)}},'*')</script>", serde_json::json!(id))
}

struct FixtureServer {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl FixtureServer {
    fn new(cross_origin: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stopped);
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/");
                let body = if path == "/parent" {
                    parent_html(cross_origin.as_deref().unwrap_or(""))
                } else {
                    child_html(path)
                };
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
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
            let _ = worker.join();
        }
    }
}

struct FixtureDirectory {
    path: PathBuf,
}
impl FixtureDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("safebrowse-print-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self { path }
    }
}
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let parent = std::env::temp_dir();
        if self.path.parent() == Some(parent.as_path())
            && self.path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("safebrowse-print-probe-")
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
