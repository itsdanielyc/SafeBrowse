# Upstream notice supplements

The Cargo archives for `webview2-com 0.38.2`, `webview2-com-macros 0.8.1`, and
`webview2-com-sys 0.38.2` declare MIT but omit the repository's license file.
The two `webview2-rs-*-LICENSE.txt` files are unchanged downloads from the exact
upstream commits recorded in each archive's `.cargo_vcs_info.json`.

The `webview2-com-sys` archive also includes Microsoft's WebView2 loader.
Its changelog and DLL version identify SDK `1.0.3650.58`. The x64 static loader
is byte-for-byte identical to `build/native/x64/WebView2LoaderStatic.lib` in
Microsoft's `Microsoft.Web.WebView2 1.0.3650.58` NuGet package. The Microsoft
`LICENSE.txt` and `NOTICE.txt` here are unchanged extracts from that package.

[supplements.json](supplements.json) records the source URLs, crate versions,
commits, SHA-256 hashes, and matching SDK archive entry. The generator verifies
the local supplement hashes, crate commits, and packaged x64 static loader
before using these files. It does not download anything. Preserve these files'
original bytes when updating them.

From the repository root, after fetching the locked Windows dependencies:

```powershell
pwsh -NoProfile -File scripts/New-ThirdPartyNotices.ps1
pwsh -NoProfile -File scripts/New-ThirdPartyNotices.ps1 -Check
```

The first command regenerates `THIRD_PARTY_NOTICES.txt`; the second compares it
without writing. The combined document uses LF line endings and preserves the
supplied text, including copyright statements. Missing packaged licenses and
mismatched supplements fail before replacing the output. Review new upstream
terms and sources when updating dependencies; this inventory is not a legal audit.
