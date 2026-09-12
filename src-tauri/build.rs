fn main() {
    tauri_build::build();

    #[cfg(windows)]
    {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let manifest_o = std::path::Path::new(&out_dir).join("manifest.o");
        let manifest_rc = std::path::Path::new(&out_dir).join("manifest.rc");
        let app_manifest = std::path::Path::new(&out_dir).join("app.manifest");

        let manifest_content = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<dependency>
    <dependentAssembly>
        <assemblyIdentity
            type="win32"
            name="Microsoft.Windows.Common-Controls"
            version="6.0.0.0"
            processorArchitecture="*"
            publicKeyToken="6595b64144ccf1df"
            language="*"
        />
    </dependentAssembly>
</dependency>
</assembly>"#;
        let _ = std::fs::write(&app_manifest, manifest_content);
        let escaped_manifest = app_manifest.to_str().unwrap().replace('\\', "/");
        let rc_content = format!("1 24 \"{}\"\n", escaped_manifest);
        let _ = std::fs::write(&manifest_rc, rc_content);

        let windres_cmd = std::process::Command::new("windres")
            .arg("-i")
            .arg(&manifest_rc)
            .arg("-o")
            .arg(&manifest_o)
            .status();

        if let Ok(s) = windres_cmd {
            if s.success() {
                println!(
                    "cargo:rustc-link-arg={}",
                    manifest_o.to_str().unwrap().replace('\\', "/")
                );
            }
        }
    }
}
