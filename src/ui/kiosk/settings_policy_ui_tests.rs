/// Verifies policy display and exact setting messages inside the real bundled settings document.
fn assert_native_download_and_printing_settings(
    event_loop: &mut EventLoop<KioskEvent>,
    window: &Window,
    context: &mut WebContext,
) {
    let view = build_test_surface(
        event_loop,
        window,
        context,
        generate_settings_page_html_with_session(false, true, true),
        Surface::Internal,
        LogicalSize::new(800.0, 600.0),
    );
    let initial = evaluate_bundled_document(event_loop, &view,
        "({downloadDisabled:document.getElementById('download-policy').disabled,printingDisabled:document.getElementById('printing-enabled').disabled,label:document.getElementById('printing-state').textContent,rows:Array.from(document.querySelectorAll('[aria-labelledby=browser-heading] .setting strong'),row=>row.textContent)})");
    assert_eq!(initial["downloadDisabled"], true);
    assert_eq!(initial["printingDisabled"], true);
    assert_eq!(initial["label"], "Loading…");
    assert_eq!(
        initial["rows"],
        json!([
            "On-screen keyboard",
            "Pop-up windows",
            "Downloads",
            "Printing"
        ])
    );
    let loaded = evaluate_bundled_document(event_loop, &view,
        "window.updatePermissions({popup_default:'ask',downloads_default:'allow',printing_enabled:true,site_rules:[]});({download:document.getElementById('download-policy').value,downloadDisabled:document.getElementById('download-policy').disabled,printing:document.getElementById('printing-enabled').checked,printingDisabled:document.getElementById('printing-enabled').disabled,label:document.getElementById('printing-state').textContent})");
    assert_eq!(loaded["download"], "allow");
    assert_eq!(loaded["downloadDisabled"], false);
    assert_eq!(loaded["printing"], true);
    assert_eq!(loaded["printingDisabled"], false);
    assert_eq!(loaded["label"], "Enabled");
    for decision in ["ask", "block", "allow"] {
        view.evaluate_script(&format!(
            "document.getElementById('download-policy').value={};document.getElementById('download-policy').dispatchEvent(new Event('change'));",
            json!(decision)
        )).unwrap();
        assert_eq!(
            wait_for_surface_command(event_loop, Surface::Internal, "SET_DOWNLOAD_POLICY"),
            json!({"type":"SET_DOWNLOAD_POLICY", "decision":decision})
        );
    }
    view.evaluate_script("document.getElementById('printing-enabled').click()")
        .unwrap();
    assert_eq!(
        wait_for_surface_command(event_loop, Surface::Internal, "SET_PRINTING_ENABLED"),
        json!({"type":"SET_PRINTING_ENABLED", "enabled":false})
    );
    view.evaluate_script("document.getElementById('printing-enabled').click()")
        .unwrap();
    assert_eq!(
        wait_for_surface_command(event_loop, Surface::Internal, "SET_PRINTING_ENABLED"),
        json!({"type":"SET_PRINTING_ENABLED", "enabled":true})
    );
    let defaults = evaluate_bundled_document(event_loop, &view,
        "window.updatePermissions({popup_default:'ask',site_rules:[]});({download:document.getElementById('download-policy').value,printing:document.getElementById('printing-enabled').checked,label:document.getElementById('printing-state').textContent})");
    assert_eq!(defaults["download"], "ask");
    assert_eq!(defaults["printing"], false);
    assert_eq!(defaults["label"], "Disabled");
}
