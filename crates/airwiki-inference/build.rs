fn main() {
    for name in [
        "AIRWIKI_MACOS_LLAMA_SERVER_SHA256",
        "AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
}
