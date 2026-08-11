fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set("ProductName", "AirWiki");
        resource.set(
            "FileDescription",
            "Provides the local read-only MCP bridge for AirWiki",
        );
        resource.compile()?;
    }
    Ok(())
}
