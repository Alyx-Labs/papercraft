//! Local HTTP dev server (`cargo xtask update-dev-server`) for testing desktop auto-updates
//! end-to-end before publishing a public GitHub Release.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use papercraft_updater::SemVer;
use papercraft_updater::manifest::{
    AppId, AssetFormat, ENV_CURRENT_VERSION, ENV_MANIFEST_URL, PlatformAsset, ReleaseManifest,
    current_platform_key,
};
use sha2::{Digest, Sha256};

const DEFAULT_PORT: u16 = 8799;
const DEFAULT_TEST_VERSION: &str = "9.9.9";
const DEFAULT_TEST_NOTES: &str = "- Local dev test update served by `cargo xtask update-dev-server`.\n- Verifies HTTPS/HTTP manifest check, SHA-256 verification, in-place replacement, and relaunch.";
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
struct DevServerOptions {
    port: u16,
    version: String,
    notes: String,
    profile: Option<String>,
    papercraft_bin: Option<PathBuf>,
    server_bin: Option<PathBuf>,
}

impl Default for DevServerOptions {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            version: DEFAULT_TEST_VERSION.to_string(),
            notes: DEFAULT_TEST_NOTES.to_string(),
            profile: None,
            papercraft_bin: None,
            server_bin: None,
        }
    }
}

#[derive(Clone)]
struct ServedAsset {
    file_name: String,
    disk_path: PathBuf,
    size_bytes: u64,
    sha256: String,
}

#[derive(Clone)]
struct ServerState {
    manifest_json: Vec<u8>,
    release_html: Vec<u8>,
    assets: BTreeMap<String, ServedAsset>,
}

/// Runs the local dev auto-update HTTP server on `127.0.0.1:<port>`.
pub fn run(args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    SemVer::parse(&opts.version).map_err(|e| e.to_string())?;

    let root = crate::workspace_root()?;
    let (papercraft_path, server_path) = resolve_binary_paths(&root, &opts)?;
    if papercraft_path.is_none() && server_path.is_none() {
        let profile_hint = opts.profile.as_deref().unwrap_or("debug");
        let target_hint = if profile_hint == "release" {
            "cargo build --release -p papercraft -p papercraft-server"
        } else {
            "cargo build -p papercraft -p papercraft-server"
        };
        return Err(format!(
            "No compiled `papercraft` or `papercraft-server` binaries found under `{}`.\n\
             Build them first with:\n  {target_hint}",
            root.join("target").display()
        ));
    }

    let listener = TcpListener::bind(("127.0.0.1", opts.port))
        .map_err(|err| format!("failed to bind 127.0.0.1:{}: {err}", opts.port))?;
    let actual_port = listener.local_addr().map_or(opts.port, |addr| addr.port());

    let state = Arc::new(build_server_state(
        actual_port,
        &opts.version,
        &opts.notes,
        papercraft_path.as_deref(),
        server_path.as_deref(),
    )?);

    print_usage_banner(
        actual_port,
        &opts.version,
        papercraft_path.as_deref(),
        server_path.as_deref(),
    );

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let state_ref = Arc::clone(&state);
        thread::spawn(move || {
            handle_connection(stream, &state_ref);
        });
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<DevServerOptions, String> {
    let mut opts = DevServerOptions::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => {
                return Err(
                    "usage: cargo xtask update-dev-server [--port 8799] [--version 9.9.9] [--notes \"...\"] [--profile debug|release] [--papercraft-bin PATH] [--server-bin PATH]"
                        .to_string(),
                );
            }
            "--port" => {
                let val = next_arg_value(args, &mut i, "--port")?;
                opts.port = val
                    .parse::<u16>()
                    .map_err(|err| format!("invalid --port `{val}`: {err}"))?;
            }
            "--version" => {
                opts.version = next_arg_value(args, &mut i, "--version")?.to_string();
            }
            "--notes" => {
                opts.notes = next_arg_value(args, &mut i, "--notes")?.to_string();
            }
            "--profile" => {
                let val = next_arg_value(args, &mut i, "--profile")?;
                if val != "debug" && val != "release" {
                    return Err(format!(
                        "invalid --profile `{val}` (expected debug or release)"
                    ));
                }
                opts.profile = Some(val.to_string());
            }
            "--papercraft-bin" => {
                opts.papercraft_bin = Some(PathBuf::from(next_arg_value(
                    args,
                    &mut i,
                    "--papercraft-bin",
                )?));
            }
            "--server-bin" => {
                opts.server_bin =
                    Some(PathBuf::from(next_arg_value(args, &mut i, "--server-bin")?));
            }
            other => {
                return Err(format!("unknown update-dev-server option `{other}`"));
            }
        }
        i += 1;
    }
    Ok(opts)
}

fn next_arg_value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value after `{flag}`"))
}

fn resolve_binary_paths(
    root: &Path,
    opts: &DevServerOptions,
) -> Result<(Option<PathBuf>, Option<PathBuf>), String> {
    let profile = match &opts.profile {
        Some(p) => p.as_str(),
        None => {
            let rel_pc = root
                .join("target")
                .join("release")
                .join(AppId::PaperCraft.executable_name());
            let rel_srv = root
                .join("target")
                .join("release")
                .join(AppId::PaperCraftServer.executable_name());
            if rel_pc.is_file() || rel_srv.is_file() {
                "release"
            } else {
                "debug"
            }
        }
    };

    let pc = match &opts.papercraft_bin {
        Some(path) => {
            if !path.is_file() {
                return Err(format!(
                    "--papercraft-bin `{}` does not exist",
                    path.display()
                ));
            }
            Some(path.clone())
        }
        None => {
            let candidate = root
                .join("target")
                .join(profile)
                .join(AppId::PaperCraft.executable_name());
            candidate.is_file().then_some(candidate)
        }
    };

    let srv = match &opts.server_bin {
        Some(path) => {
            if !path.is_file() {
                return Err(format!("--server-bin `{}` does not exist", path.display()));
            }
            Some(path.clone())
        }
        None => {
            let candidate = root
                .join("target")
                .join(profile)
                .join(AppId::PaperCraftServer.executable_name());
            candidate.is_file().then_some(candidate)
        }
    };

    Ok((pc, srv))
}

fn dev_asset_format() -> AssetFormat {
    if cfg!(target_os = "windows") {
        AssetFormat::Exe
    } else {
        AssetFormat::Binary
    }
}

fn build_server_state(
    port: u16,
    version: &str,
    notes: &str,
    papercraft_bin: Option<&Path>,
    server_bin: Option<&Path>,
) -> Result<ServerState, String> {
    let base_url = format!("http://127.0.0.1:{port}");
    let platform = current_platform_key();
    let format = dev_asset_format();
    let clean_version = version.trim().trim_start_matches(['v', 'V']);
    let tag = format!("v{clean_version}");

    let mut manifest_assets: BTreeMap<String, BTreeMap<String, PlatformAsset>> = BTreeMap::new();
    let mut served_assets: BTreeMap<String, ServedAsset> = BTreeMap::new();

    for (app, maybe_path) in [
        (AppId::PaperCraft, papercraft_bin),
        (AppId::PaperCraftServer, server_bin),
    ] {
        let Some(path) = maybe_path else {
            continue;
        };
        let (sha256, size_bytes) = hash_file_sha256(path)?;
        let file_name = format!(
            "{}-{platform}-dev{}",
            app.manifest_key(),
            if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            }
        );
        let url = format!("{base_url}/assets/{file_name}");

        let mut per_platform = BTreeMap::new();
        per_platform.insert(
            platform.clone(),
            PlatformAsset {
                file_name: file_name.clone(),
                url,
                sha256: sha256.clone(),
                size_bytes,
                format,
            },
        );
        manifest_assets.insert(app.manifest_key().to_string(), per_platform);
        served_assets.insert(
            file_name.clone(),
            ServedAsset {
                file_name,
                disk_path: path.to_path_buf(),
                size_bytes,
                sha256,
            },
        );
    }

    let manifest = ReleaseManifest {
        schema_version: 1,
        version: clean_version.to_string(),
        tag: tag.clone(),
        release_url: format!("{base_url}/release"),
        published_at: "2026-01-01T00:00:00Z".to_string(),
        notes: notes.to_string(),
        assets: manifest_assets,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| format!("failed serializing dev manifest JSON: {err}"))?;
    let release_html = format!(
        "<!doctype html><html><body><h1>PaperCraft Dev Release {tag}</h1><p>Manifest: <a href=\"/latest.json\">/latest.json</a></p></body></html>"
    )
    .into_bytes();

    Ok(ServerState {
        manifest_json,
        release_html,
        assets: served_assets,
    })
}

fn hash_file_sha256(path: &Path) -> Result<(String, u64), String> {
    let mut file =
        File::open(path).map_err(|err| format!("failed opening `{}`: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut buf = vec![0_u8; STREAM_CHUNK_BYTES];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|err| format!("failed reading `{}`: {err}", path.display()))?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        hasher.update(&buf[..n]);
    }
    Ok((lower_hex(&hasher.finalize()), total))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0F)]));
    }
    out
}

fn handle_connection(mut stream: TcpStream, state: &ServerState) {
    let mut buf = [0_u8; 4096];
    let Ok(n) = stream.read(&mut buf) else {
        return;
    };
    if n == 0 {
        return;
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let mut parts = req.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_target = parts.next().unwrap_or("/");
    let path = raw_target.split('?').next().unwrap_or("/");

    if method != "GET" && method != "HEAD" {
        let _ = write_simple_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"method not allowed",
            false,
        );
        return;
    }
    let head_only = method == "HEAD";

    if path == "/latest.json" {
        let _ = write_simple_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            &state.manifest_json,
            head_only,
        );
        return;
    }

    if path == "/release" || path == "/" {
        let _ = write_simple_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            &state.release_html,
            head_only,
        );
        return;
    }

    if let Some(asset_name) = path.strip_prefix("/assets/")
        && let Some(asset) = state.assets.get(asset_name)
    {
        println!(
            "==> Serving {} ({} bytes, sha256={})",
            asset.file_name, asset.size_bytes, asset.sha256
        );
        let _ = write_file_response(&mut stream, asset, head_only);
        return;
    }

    let _ = write_simple_response(
        &mut stream,
        "404 Not Found",
        "text/plain",
        b"not found",
        head_only,
    );
}

fn write_simple_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn write_file_response(
    stream: &mut TcpStream,
    asset: &ServedAsset,
    head_only: bool,
) -> std::io::Result<()> {
    let meta = fs::metadata(&asset.disk_path)?;
    let len = meta.len();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {len}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(header.as_bytes())?;
    if head_only {
        return stream.flush();
    }
    let mut file = File::open(&asset.disk_path)?;
    let mut buf = vec![0_u8; STREAM_CHUNK_BYTES];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
    }
    stream.flush()
}

fn print_usage_banner(
    port: u16,
    version: &str,
    papercraft_bin: Option<&Path>,
    server_bin: Option<&Path>,
) {
    let manifest_url = format!("http://127.0.0.1:{port}/latest.json");
    println!("==> PaperCraft Local Update Dev Server listening on {manifest_url}");
    println!("    Serving release version: v{version}");
    if let Some(p) = papercraft_bin {
        println!("    papercraft binary:        {}", p.display());
    }
    if let Some(s) = server_bin {
        println!("    papercraft-server binary: {}", s.display());
    }
    println!();
    println!(
        "How to test safely (copy the binary to a scratch folder first so `target/` is untouched):"
    );
    if cfg!(target_os = "windows") {
        println!("  PowerShell:");
        println!("    mkdir $env:TEMP\\papercraft-update-test -Force");
        if let Some(p) = papercraft_bin {
            println!(
                "    Copy-Item \"{}\" \"$env:TEMP\\papercraft-update-test\\papercraft.exe\" -Force",
                p.display()
            );
        }
        println!("    $env:{ENV_MANIFEST_URL}=\"{manifest_url}\"");
        println!("    $env:{ENV_CURRENT_VERSION}=\"0.1.0\"");
        println!("    & \"$env:TEMP\\papercraft-update-test\\papercraft.exe\"");
    } else {
        println!("  Shell:");
        println!("    mkdir -p /tmp/papercraft-update-test");
        if let Some(p) = papercraft_bin {
            println!(
                "    cp \"{}\" /tmp/papercraft-update-test/papercraft",
                p.display()
            );
        }
        println!(
            "    {ENV_MANIFEST_URL}=\"{manifest_url}\" {ENV_CURRENT_VERSION}=\"0.1.0\" /tmp/papercraft-update-test/papercraft"
        );
    }
    println!();
    println!("Press Ctrl+C to stop the dev server.");
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use papercraft_updater::{CheckOutcome, UpdateConfig, check_for_update};

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDirGuard {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("pc_xtask_upd_{label}_{nanos}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDirGuard(p)
    }

    #[test]
    fn builds_valid_manifest_and_serves_assets_over_loopback_http() {
        let tmp = temp_dir("serve");
        let fake_pc = tmp.0.join("papercraft-bin");
        fs::write(&fake_pc, b"test-papercraft-payload").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(
            build_server_state(port, "9.9.9", "Local dev check", Some(&fake_pc), None).unwrap(),
        );

        let state_srv = Arc::clone(&state);
        let srv = thread::spawn(move || {
            for _ in 0..2 {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                handle_connection(stream, &state_srv);
            }
        });

        let cfg = UpdateConfig {
            app: AppId::PaperCraft,
            current_version: SemVer::parse("0.1.1").unwrap(),
            manifest_url: format!("http://127.0.0.1:{port}/latest.json"),
            is_custom_feed: true,
        };
        let outcome = check_for_update(&cfg).unwrap();
        let update = match outcome {
            CheckOutcome::UpdateAvailable(u) => *u,
            other => panic!("expected UpdateAvailable, got {other:?}"),
        };
        assert_eq!(update.version.to_string(), "9.9.9");
        assert_eq!(
            update.asset.size_bytes,
            b"test-papercraft-payload".len() as u64
        );

        let _ = TcpStream::connect(("127.0.0.1", port));
        let _ = srv.join();
    }
}
