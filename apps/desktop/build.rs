fn main() {
    const WINDOWS_ICON: &str = "../../resources/branding/airwiki.ico";
    println!("cargo:rerun-if-changed={WINDOWS_ICON}");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(WINDOWS_ICON);
        resource.set("ProductName", "AirWiki");
        resource.set("FileDescription", "AirWiki local-first knowledge desktop");
        resource.set("LegalCopyright", "Copyright 2026 AirWiki contributors");
        if let Err(error) = resource.compile() {
            panic!("failed to compile Windows resources: {error}");
        }
    }
}
