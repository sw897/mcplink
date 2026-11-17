fn main() {
    println!("cargo:rerun-if-changed=updater.pub");
    let key = std::fs::read_to_string("updater.pub").expect("missing update public key");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read("tauri.conf.json").expect("missing desktop config"))
            .expect("invalid desktop config");
    assert_eq!(
        config["plugins"]["updater"]["pubkey"].as_str(),
        Some(key.trim()),
        "update public key/config mismatch"
    );
    tauri_build::build()
}
