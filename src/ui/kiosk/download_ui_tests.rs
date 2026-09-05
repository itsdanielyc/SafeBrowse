/// Checks that untrusted download metadata stays text and decisions carry the displayed ID only.
fn assert_native_download_confirmation(
    event_loop: &mut EventLoop<KioskEvent>,
    window: &Window,
    context: &mut WebContext,
) {
    let view = build_test_surface(
        event_loop, window, context,
        crate::ui::assets::generate_download_prompt_html(),
        Surface::DownloadPrompt,
        LogicalSize::new(460.0, 350.0),
    );
    let initial = evaluate_bundled_document(event_loop, &view,
        "Array.from(document.querySelectorAll('button'), button => button.disabled)");
    assert_eq!(initial, json!([true, true]));
    let file_name = "<img src=x onerror='window.compromised=true'>.txt";
    let url = format!("https://files.example.test/{}", "long-path/".repeat(160));
    let request = json!({"id":17,"file_name":file_name,"origin":"https://page.example.test","url":url,"total_bytes":null});
    let shown = evaluate_bundled_document(event_loop, &view, &format!(
        "window.showRequest({request}); ({{name:document.getElementById('filename').textContent,images:document.images.length,size:document.getElementById('size').textContent,focus:document.activeElement.id,buttonsVisible:Array.from(document.querySelectorAll('button'),button=>{{const bounds=button.getBoundingClientRect();return bounds.top>=0 && bounds.bottom<=innerHeight;}})}})"
    ));
    assert_eq!(shown["name"], file_name);
    assert_eq!(shown["images"], 0);
    assert_eq!(shown["size"], "Size unknown");
    assert_eq!(shown["focus"], "cancel");
    assert_eq!(shown["buttonsVisible"], json!([true, true]));
    view.evaluate_script("document.getElementById('download').click()") .unwrap();
    assert_eq!(wait_for_surface_command(event_loop, Surface::DownloadPrompt, "RESOLVE_DOWNLOAD"), json!({"type":"RESOLVE_DOWNLOAD","id":17,"allow":true}));
    let disabled = evaluate_bundled_document(event_loop, &view,
        "Array.from(document.querySelectorAll('button'), button => button.disabled)");
    assert_eq!(disabled, json!([true, true]));
    view.evaluate_script("window.showRequest({id:18,file_name:'sample.txt',origin:'https://example.test',url:'https://example.test/sample.txt',total_bytes:0});document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));").unwrap();
    assert_eq!(wait_for_surface_command(event_loop, Surface::DownloadPrompt, "RESOLVE_DOWNLOAD"), json!({"type":"RESOLVE_DOWNLOAD","id":18,"allow":false}));
}
