fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set("ProductName", "AirWiki");
        resource.set("FileDescription", "AirWiki local-first knowledge desktop");
        resource.set("LegalCopyright", "Copyright 2026 AirWiki contributors");
        if let Err(error) = resource.compile() {
            panic!("failed to compile Windows resources: {error}");
        }
    }
}
