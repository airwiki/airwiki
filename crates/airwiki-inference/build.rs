fn main() {
    const NAME: &str = "AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256";
    println!("cargo:rerun-if-env-changed={NAME}");
}
