fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets/branding/papercraft.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resources = winres::WindowsResource::new();
        resources
            .set_icon("assets/branding/papercraft.ico")
            .set("ProductName", "PaperCraft")
            .set("FileDescription", "PaperCraft")
            .set("OriginalFilename", "papercraft.exe");
        resources.compile()?;
    }

    Ok(())
}
