# SafeBrowse artwork

These assets use the three owner-supplied PNGs from 4 September 2026:

- `safebrowse-mark.png`: square monochrome mark, from `03CABA15-091B-4358-9758-4A5E6696C4D3.png`. Transparent margins are trimmed, the aspect ratio is preserved, and the artwork is centred on a transparent 128px square.
- `safebrowse-wordmark.png`: horizontal monochrome logo, from `SafeBrowse-first-pass-monochrome.png`. Transparent margins are trimmed and the artwork is resized proportionally to 512px wide.
- `safebrowse-app.png`: colour application icon, from `17D2E2B2-B7DA-4176-B246-4E3EEB5EBD94.png`. A centred 1024px square crop removes excess transparent padding from the 1254px source, then scales to 256px while preserving the artwork's original centre.
- `safebrowse.ico`: the colour artwork packaged at 16, 20, 24, 32, 40, 48, 64, 128 and 256px for Windows.

The supplied designs are retained; only bounds, size, metadata and file format are normalized. Monochrome assets were prepared with ImageMagick 7 using `-trim +repage`, proportional `-resize`, and `-strip`. The application PNG uses `-gravity center -crop 1024x1024+0+0 +repage -resize 256x256 -strip`. Keeping its original centre avoids shifting the mark because of faint stray pixels in the supplied transparency. The ICO uses `-define icon:auto-resize=256,128,64,48,40,32,24,20,16`.

The trusted HTML embeds PNG data URLs through `src/ui/branding.rs`; it needs no network or adjacent files. The titlebar wordmark is 24px tall. The session taskbar uses a 28px mark inside a centred 44×38px static container, with no click action, hover treatment or underline. Native browser and companion windows load the colour icon from resource 1, also used by Explorer for the executable.

`build.rs` compiles `assets/safebrowse.rc` with the Windows SDK and embeds the icon in binaries, examples and test hosts. A normal `cargo build --release --locked` is sufficient; image tooling is only needed when replacing artwork. Refresh `cargo run --example ui_preview --locked` to review layout changes.
