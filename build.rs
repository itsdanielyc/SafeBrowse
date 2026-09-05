//! Embeds Windows branding and version metadata from the Cargo package version.

/// Requires resource compilation so every Windows executable has matching product metadata.
fn main() {
    const RESOURCE_FILE: &str = "assets/safebrowse.rc";
    const ICON_FILE: &str = "assets/branding/safebrowse.ico";

    println!("cargo:rerun-if-changed={RESOURCE_FILE}");
    println!("cargo:rerun-if-changed={ICON_FILE}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version_components: Vec<u16> = ["MAJOR", "MINOR", "PATCH"]
            .into_iter()
            .map(|component| {
                std::env::var(format!("CARGO_PKG_VERSION_{component}"))
                    .expect("Cargo must provide package version components")
                    .parse()
                    .expect("Windows executable version components must fit in 16 bits")
            })
            .collect();
        let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo must provide a version");
        let resource_macros = [
            format!(
                "SAFEBROWSE_VERSION_NUMBERS={},{},{},0",
                version_components[0], version_components[1], version_components[2]
            ),
            format!("SAFEBROWSE_VERSION_STRING=\"{version}\""),
        ];
        embed_resource::compile_for_everything(RESOURCE_FILE, resource_macros)
            .manifest_required()
            .expect("Cannot embed SafeBrowse Windows resources; install the Windows SDK");
    }
}
