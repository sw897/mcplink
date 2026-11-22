#[cfg(all(debug_assertions, any(windows, target_os = "macos")))]
#[tokio::main]
async fn main() {
    use mcplink_desktop::{UPDATE_PUBLIC_KEY, cli_sync, update_catalog::Catalog};
    use serde_json::{Value, json};
    use std::{fs, io::Read, path::PathBuf, time::Duration};
    let result = async {
        let mut input = String::new();
        std::io::stdin()
            .take(256 * 1024)
            .read_to_string(&mut input)
            .map_err(|_| "Invalid stdin")?;
        let args: Value = serde_json::from_str(&input).map_err(|_| "Invalid JSON")?;
        let get = |key: &str| args[key].as_str().ok_or_else(|| format!("Missing {key}"));
        if args["action"] == "desktop-rpc-contract" {
            mcplink_desktop::desktop_rpc::validate(get("method")?, &args["params"])?;
            return Ok(json!({"accepted": true}));
        }
        #[cfg(target_os = "macos")]
        if args["action"] == "keychain-update-preflight" {
            let home = PathBuf::from(get("home")?)
                .canonicalize()
                .map_err(|_| "Missing fixture")?;
            if !home.join(".mcplink-test-fixture").is_file() {
                return Err("Missing fixture marker".into());
            }
            let data = PathBuf::from(get("data")?)
                .canonicalize()
                .map_err(|_| "Missing fixture data")?;
            let package = PathBuf::from(get("package")?)
                .canonicalize()
                .map_err(|_| "Missing fixture package")?;
            if !data.starts_with(&home) || !package.starts_with(&home) {
                return Err("Preflight inputs outside fixture".into());
            }
            let bytes = fs::read(package).map_err(|_| "Cannot read fixture package")?;
            let _guard = mcplink_desktop::macos_update::validate_package(&data, &bytes)?;
            if let Some(ms) = args["holdMs"].as_u64().filter(|ms| *ms <= 5000) {
                use std::io::Write;
                println!("{}", json!({"locked":true}));
                std::io::stdout()
                    .flush()
                    .map_err(|_| "Fixture output failed")?;
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            return Ok(json!({"verified":true}));
        }
        let current = get("version")?
            .parse()
            .map_err(|_| "Invalid current version")?;
        if matches!(
            args["action"].as_str(),
            Some("sync" | "discover" | "archive")
        ) {
            let home = PathBuf::from(get("home")?);
            if !home.join(".mcplink-test-fixture").is_file() {
                return Err("Missing fixture marker".into());
            }
            let sidecar = PathBuf::from(get("sidecar")?);
            if !sidecar
                .canonicalize()
                .map_err(|_| "Missing sidecar")?
                .starts_with(home.canonicalize().map_err(|_| "Missing home")?)
            {
                return Err("Sidecar outside fixture".into());
            }
            let receipt = home.join("desktop-updates/pending.json");
            return match args["action"].as_str() {
                Some("discover") => Ok(json!({"targets": cli_sync::discover(&home, &sidecar)})),
                Some("archive") => {
                    cli_sync::archive_stale_receipt(&receipt, &current)?;
                    Ok(json!({"archived": true}))
                }
                _ => Ok(json!({"synced": cli_sync::apply_receipt(
                    &receipt, &home, &sidecar, &current
                )?})),
            };
        }
        let target = get("target")?.to_owned();
        let catalog = if args["live"] == true {
            Catalog::github(current, target, UPDATE_PUBLIC_KEY.trim().into())
        } else {
            let home = PathBuf::from(get("home")?);
            let key = PathBuf::from(get("publicKey")?);
            if !key
                .canonicalize()
                .map_err(|_| "Missing public key")?
                .starts_with(home.canonicalize().map_err(|_| "Missing home")?)
            {
                return Err("Public key outside fixture".into());
            }
            Catalog::fixture(
                current,
                target,
                fs::read_to_string(key)
                    .map_err(|_| "Missing public key")?
                    .trim()
                    .into(),
                &home,
                get("api")?.parse().map_err(|_| "Invalid API")?,
                get("downloads")?.parse().map_err(|_| "Invalid downloads")?,
            )?
        };
        let Some(release) = catalog
            .latest(args["preview"].as_bool().unwrap_or(true))
            .await?
        else {
            return Ok(json!({"phase": "no-release"}));
        };
        if !release.version.cmp_precedence(&catalog.current).is_gt() {
            return Ok(json!({"phase": "current", "release": release}));
        }
        let Some(package) = catalog.package(&release).await? else {
            return Ok(json!({"phase": "unavailable", "release": release}));
        };
        let mut result = json!({"phase": "available", "release": release, "package": {
            "version": package.version, "sha256": package.sha256, "bytes": package.bytes
        }});
        if args["action"] == "download" {
            let (cancel, receiver) = tokio::sync::watch::channel(false);
            let task_cancel = cancel.clone();
            let task = args["cancelAfterMs"].as_u64().map(|ms| {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    task_cancel.send_replace(true);
                })
            });
            let mut progress = Vec::new();
            let bytes = catalog
                .download(&package, receiver, |done, total| {
                    progress.push((done, total));
                })
                .await?;
            if let Some(task) = task {
                task.abort();
            }
            drop(cancel);
            result["phase"] = json!("verified");
            result["bytes"] = json!(bytes.len());
            result["progressEvents"] = json!(progress.len());
        }
        Ok::<_, String>(result)
    }
    .await;
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            println!("{}", serde_json::json!({"error":error}));
            std::process::exit(2);
        }
    }
}

#[cfg(not(all(debug_assertions, any(windows, target_os = "macos"))))]
fn main() {
    eprintln!("The updater contract driver is only available in debug fixture builds");
    std::process::exit(2);
}
