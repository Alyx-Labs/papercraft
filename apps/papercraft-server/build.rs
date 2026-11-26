fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../papercraft/assets/branding/papercraft.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resources = winres::WindowsResource::new();
        resources
            .set_language(0x0409)
            // TODO(branding): Replace ../papercraft/assets/branding/papercraft.ico with a dedicated
            // PaperCraft Server .ico once server-specific logo artwork is finalized.
            .set_icon("../papercraft/assets/branding/papercraft.ico")
            .set("ProductName", "PaperCraft Server")
            // Displayed in Windows Task Manager (Processes & Startup apps Name column).
            .set("FileDescription", "PaperCraft Server")
            // Displayed in Windows Task Manager (Startup apps Publisher column) & Explorer Properties.
            .set("CompanyName", "Alyx Labs")
            // TODO(metadata): Set "LegalTrademark" if Alyx Labs registers a trademark, and update the
            // copyright year / legal entity suffix as needed.
            .set("LegalCopyright", "Copyright (c) 2026 Alyx Labs")
            .set("OriginalFilename", "papercraft-server.exe")
            .set("InternalName", "papercraft-server")
            .set(
                "Comments",
                "Portable standalone self-hosted sync server and system-tray control center for PaperCraft by Alyx Labs.",
            );
        // TODO(signing): Sign release .exe binaries with an Alyx Labs Authenticode certificate so
        // Windows SmartScreen and Task Manager show a cryptographically verified publisher.
        resources.compile()?;
    }

    Ok(())
}
