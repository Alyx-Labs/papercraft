#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod branding;
mod layers;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn is_workspace_root(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file() && dir.join("apps").join("papercraft-web").is_dir()
}

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR")
        && let Some(parent) = Path::new(&manifest_dir).parent()
        && is_workspace_root(parent)
    {
        return Ok(parent.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir()
        && let Some(found) = cwd.ancestors().find(|dir| is_workspace_root(dir))
    {
        return Ok(found.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(found) = exe.ancestors().find(|dir| is_workspace_root(dir))
    {
        return Ok(found.to_path_buf());
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .filter(|dir| is_workspace_root(dir))
        .ok_or_else(|| "failed to locate workspace root directory".to_string())
}

fn run_cargo_step(label: &str, args: &[&str]) -> Result<(), String> {
    println!("==> {label}");
    let status = Command::new("cargo")
        .args(args)
        .status()
        .map_err(|err| format!("failed to start `cargo {}`: {err}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with status {status}"))
    }
}

fn web_app_dir() -> Result<PathBuf, String> {
    Ok(workspace_root()?.join("apps").join("papercraft-web"))
}

fn run_trunk(subcommand: &str, default_args: &[&str], extra_args: &[String]) -> Result<(), String> {
    let web_dir = web_app_dir()?;
    let mut cmd = Command::new("trunk");
    cmd.current_dir(&web_dir).arg(subcommand);
    if extra_args.is_empty() {
        cmd.args(default_args);
    } else {
        cmd.args(extra_args);
    }
    println!("==> Running `trunk {subcommand}` in {}", web_dir.display());
    let status = cmd.status().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            format!(
                "failed to start `trunk {subcommand}`: `trunk` executable not found ({err})\n\
                 Install WebAssembly prerequisites first:\n\
                 rustup target add wasm32-unknown-unknown\n\
                 cargo install --locked trunk"
            )
        } else {
            format!(
                "failed to start `trunk {subcommand}` in `{}`: {err}",
                web_dir.display()
            )
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`trunk {subcommand}` exited with status {status}"))
    }
}

fn run_ci() -> Result<(), String> {
    let count = layers::run()?;
    println!("layers ok ({count} workspace crates checked)");
    run_cargo_step("Checking formatting", &["fmt", "--all", "--", "--check"])?;
    run_cargo_step(
        "Running Clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_cargo_step("Running workspace tests", &["test", "--workspace"])?;
    println!("ci ok");
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let subcommand = args.next();
    let rest: Vec<String> = args.collect();
    match subcommand.as_deref() {
        Some("layers") => match layers::run() {
            Ok(count) => {
                println!("layers ok ({count} workspace crates checked)");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Some("branding") => match branding::run() {
            Ok(count) => {
                println!("generated {count} branding assets");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Some("ci") => match run_ci() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Some("web") => match run_trunk("serve", &[], &rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Some("web-build") => match run_trunk("build", &["--release"], &rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Some(other) => {
            eprintln!(
                "unknown xtask command `{other}` (expected: layers, branding, ci, web, or web-build)"
            );
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: cargo xtask <layers|branding|ci|web|web-build>");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn web_app_dir_resolves_existing_directory() {
        let dir = web_app_dir().unwrap();
        assert!(dir.is_dir(), "expected web app dir at {}", dir.display());
    }
}
