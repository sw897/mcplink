fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    // codesign otherwise derives a CLI identity from the filename/Mach-O UUID.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let path =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("cli-info.plist");
    std::fs::write(&path, format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.mcplink.cli</string>
<key>CFBundleName</key><string>mcplink</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>MCPLinkKeychainBindingVersion</key><integer>2</integer>
</dict></plist>
"#)).unwrap();
    println!(
        "cargo:rustc-link-arg-bin=mcplink=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        path.display()
    );
}
