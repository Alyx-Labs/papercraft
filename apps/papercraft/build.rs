fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/branding/papercraft.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resources = winres::WindowsResource::new();
        resources
            .set_language(0x0409)
            .set_icon("assets/branding/papercraft.ico")
            .set("ProductName", "PaperCraft")
            .set("FileDescription", "PaperCraft")
            .set("CompanyName", "Alyx Labs")
            // TODO(metadata): Set "LegalTrademark" if Alyx Labs registers a trademark, and update the
            // copyright year / legal entity suffix as needed.
            .set("LegalCopyright", "Copyright (c) 2026 Alyx Labs")
            .set("OriginalFilename", "papercraft.exe")
            .set("InternalName", "papercraft")
            .set(
                "Comments",
                "Portable standalone notebook and handwriting studio for Windows by Alyx Labs.",
            );
        // TODO(signing): Sign release .exe binaries with an Alyx Labs Authenticode certificate so
        // Windows SmartScreen and Task Manager show a cryptographically verified publisher.
        resources.compile()?;
    }

    Ok(())
}
