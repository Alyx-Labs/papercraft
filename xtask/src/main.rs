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

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .ok_or_else(|| "failed to locate workspace root from xtask manifest dir".to_string())?;
    Ok(root.join("apps").join("papercraft-web"))
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
        format!(
            "failed to start `trunk {subcommand}`: {err}\n\
             Install WebAssembly prerequisites first:\n\
             rustup target add wasm32-unknown-unknown\n\
             cargo install --locked trunk"
        )
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
