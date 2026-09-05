/// Exercises the real bundled chrome DOM without opening a print dialog or contacting a website.
fn assert_native_print_controls(
    event_loop: &mut EventLoop<KioskEvent>,
    window: &Window,
    context: &mut WebContext,
) {
    use crate::browser::tabs::TabItem;

    const FIRST_TAB_ID: usize = 7;
    const SECOND_TAB_ID: usize = 19;
    const INTERNAL_TAB_ID: usize = 23;
    let first_tab = TabItem::new(FIRST_TAB_ID, "https://example.invalid/first");
    let second_tab = TabItem::new(SECOND_TAB_ID, "https://example.invalid/second");
    let internal_tab = TabItem::new_special(INTERNAL_TAB_ID, "Settings", TabKind::Settings);
    let tabs = [first_tab, second_tab, internal_tab];
    let view = build_test_surface(
        event_loop,
        window,
        context,
        generate_browser_chrome_html_with_session(&tabs, FIRST_TAB_ID, false, true),
        Surface::Chrome,
        LogicalSize::new(800.0, BROWSER_CHROME_HEIGHT),
    );
    let button = evaluate_bundled_document(event_loop, &view,
        "(() => {const button = document.getElementById('print-btn'); return {disabled:button.disabled,label:button.getAttribute('aria-label'),shortcut:button.getAttribute('aria-keyshortcuts'),title:button.title,command:button.dataset.command};})()");
    assert_eq!(button["disabled"], false);
    assert_eq!(button["label"], "Print");
    assert_eq!(button["shortcut"], "Control+p");
    assert_eq!(button["title"], "Print (Ctrl+P)");
    assert_eq!(button["command"], "PRINT");
    view.evaluate_script("document.getElementById('print-btn').click()")
        .unwrap();
    assert_eq!(
        wait_for_surface_command(event_loop, Surface::Chrome, "PRINT")["id"],
        FIRST_TAB_ID
    );

    // Record at the common transport boundary to detect duplicate generic and tab-bound handlers.
    let exact_shortcuts = evaluate_bundled_document(
        event_loop,
        &view,
        r#"
        (() => {
            window.__printTestMessages = [];
            postIpc = message => window.__printTestMessages.push(message);
            window.__printTestStroke = options => {
                const event = new KeyboardEvent('keydown', {key:'p',ctrlKey:true,bubbles:true,cancelable:true,...options});
                window.dispatchEvent(event);
                return event.defaultPrevented;
            };
            document.getElementById('print-btn').click();
            const clicked = window.__printTestMessages.splice(0);
            const accepted = window.__printTestStroke({});
            const pressed = window.__printTestMessages.splice(0);
            const repeated = window.__printTestStroke({repeat:true});
            const repeatedMessages = window.__printTestMessages.splice(0);
            const rejected = [{ctrlKey:false},{altKey:true},{shiftKey:true},{metaKey:true}].map(window.__printTestStroke);
            const rejectedMessages = window.__printTestMessages.splice(0);
            window.__printTestStroke({key:'P'});
            return {clicked,accepted,pressed,repeated,repeatedMessages,rejected,rejectedMessages,uppercase:window.__printTestMessages.splice(0)};
        })()
    "#,
    );
    let first_request = json!([{ "type": "PRINT", "id": FIRST_TAB_ID }]);
    assert_eq!(exact_shortcuts["clicked"], first_request);
    assert_eq!(exact_shortcuts["accepted"], true);
    assert_eq!(exact_shortcuts["pressed"], first_request);
    assert_eq!(exact_shortcuts["repeated"], true);
    assert_eq!(exact_shortcuts["repeatedMessages"], json!([]));
    assert_eq!(
        exact_shortcuts["rejected"],
        json!([false, false, false, false])
    );
    assert_eq!(exact_shortcuts["rejectedMessages"], json!([]));
    assert_eq!(exact_shortcuts["uppercase"], first_request);

    for active_id in [INTERNAL_TAB_ID, usize::MAX] {
        let disabled = evaluate_bundled_document(event_loop, &view, &format!(
            "(() => {{window.updateTabs({}, {active_id}); document.getElementById('print-btn').click(); window.__printTestStroke({{}}); return {{disabled:document.getElementById('print-btn').disabled,messages:window.__printTestMessages.splice(0)}};}})()",
            json!(tabs)
        ));
        assert_eq!(disabled["disabled"], true);
        assert_eq!(disabled["messages"], json!([]));
    }
    let switched = evaluate_bundled_document(event_loop, &view, &format!(
        "(() => {{window.updateTabs({}, {SECOND_TAB_ID}); document.getElementById('print-btn').click(); return {{disabled:document.getElementById('print-btn').disabled,messages:window.__printTestMessages.splice(0)}};}})()",
        json!(tabs)
    ));
    assert_eq!(switched["disabled"], false);
    assert_eq!(
        switched["messages"],
        json!([{ "type": "PRINT", "id": SECOND_TAB_ID }])
    );
}
