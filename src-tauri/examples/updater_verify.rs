#[cfg(any(windows, target_os = "macos"))]
fn main() {
    use mcplink_desktop::update_signature;
    use std::{fs, path::Path};
    let result = (|| -> Result<serde_json::Value, String> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 4 {
            return Err("Usage: updater_verify PUBLIC_KEY FILE SIGNATURE VERSION".into());
        }
        let key = fs::read_to_string(&args[0]).map_err(|_| "Cannot read public key")?;
        let bytes = fs::read(&args[1]).map_err(|_| "Cannot read update package")?;
        let signature = fs::read_to_string(&args[2]).map_err(|_| "Cannot read signature")?;
        let version = args[3].parse().map_err(|_| "Invalid version")?;
        update_signature::verify(&bytes, &signature, &key, &version)?;
        Ok(
            serde_json::json!({"verified":true,"version":args[3],"name":Path::new(&args[1]).file_name().map(|n|n.to_string_lossy().into_owned()),"sha256":update_signature::sha256(&bytes),"bytes":bytes.len()}),
        )
    })();
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", serde_json::json!({"error":error}));
            std::process::exit(2);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("Updater release verification is supported on macOS and Windows");
    std::process::exit(2);
}
