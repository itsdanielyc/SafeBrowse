# Preview the interface

The UI preview exports SafeBrowse's bundled HTML and sample data for reviewing layout and accessibility in an ordinary browser. It does not start SafeBrowse, launch WebView2, create an isolated desktop, or change screen-capture policy.

From the repository root:

```powershell
cargo run --example ui_preview
python -m http.server 8765 --bind 127.0.0.1 --directory target/ui-preview
```

Open [the local preview](http://127.0.0.1:8765/index.html). Keep the server bound to `127.0.0.1`; all preview assets are local. Stop the server with Ctrl+C when finished.

The preview uses the same 110px browser controls, 230px keyboard, and 46px taskbar as the application. It starts with the red recording warning. Clicking **OK** dismisses that preview overlay. **Settings**, **Bookmarks**, tab controls, bookmark search, and the keyboard can be inspected in context. Sample bookmark domains use the reserved `example.com` domain.

Native actions display a preview notice. Websites do not load, bookmark edits do not persist, and the preview does not test desktop isolation or Windows capture exclusion. The generated child pages can also be opened separately to inspect narrow layouts.

After changing templates in `src/ui/web` or the Rust presentation generators in `src/ui/assets.rs`, rerun the exporter and refresh the browser.
