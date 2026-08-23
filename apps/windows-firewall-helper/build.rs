fn main() {
    println!("cargo:rerun-if-env-changed=AIRWIKI_WINDOWS_SIGNER_SHA256");
    #[cfg(windows)]
    {
        const MANIFEST: &str = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

        let mut resource = winresource::WindowsResource::new();
        resource.set("ProductName", "AirWiki");
        resource.set(
            "FileDescription",
            "Configures restricted local-network firewall rules for AirWiki",
        );
        resource.set_manifest(MANIFEST);
        if let Err(error) = resource.compile() {
            panic!("failed to compile Windows firewall helper resources: {error}");
        }
    }
}
