fn main() {
    // On Windows the executable carries the app icon; elsewhere this does nothing.
    println!("cargo:rerun-if-changed=assets/gdirect.rc");
    println!("cargo:rerun-if-changed=assets/gdirect.ico");
    embed_resource::compile("assets/gdirect.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
