fn main() {
    #[cfg(windows)]
    {
        let assets = std::path::Path::new("assets");
        println!("cargo:rerun-if-changed=assets/app-icon.rc");
        println!("cargo:rerun-if-changed=assets/app-icon.ico");
        embed_resource::compile(
            assets.join("app-icon.rc"),
            embed_resource::ParamsIncludeDirs([assets]),
        )
        .manifest_optional()
        .expect("failed to embed Panda Reader icon");
    }
}
