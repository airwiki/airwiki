fn main() -> Result<(), Box<dyn std::error::Error>> {
    for name in [
        "AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES",
        "AIRWIKI_UPDATE_ENDPOINT",
        "AIRWIKI_UPDATER_PUBLIC_KEY",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let attributes = tauri_build::Attributes::new();
    #[cfg(target_os = "windows")]
    let attributes = attributes.windows_attributes(
        tauri_build::WindowsAttributes::new()
            .window_icon_path("../../resources/branding/airwiki.ico"),
    );
    tauri_build::try_build(attributes)?;
    Ok(())
}
