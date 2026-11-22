fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../papercraft/assets/branding/papercraft.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resources = winres::WindowsResource::new();
        resources
            .set_icon("../papercraft/assets/branding/papercraft.ico")
            .set("ProductName", "PaperCraft Server")
            .set("FileDescription", "PaperCraft Sync Server Control")
            .set("OriginalFilename", "papercraft-server.exe");
        resources.compile()?;
    }

    Ok(())
}
