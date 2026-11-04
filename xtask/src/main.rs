//! `cargo xtask <task>` — developer tasks.
//!
//! - `ci`: all quality gates (fmt, clippy, test). Run before every commit.
//! - `dist`: the release archive for this platform, in `dist/` (see `dist.rs`).

#![forbid(unsafe_code)]

mod dist;

use std::env;
use std::process::{Command, ExitCode};

const GATES: &[(&str, &[&str])] = &[
    ("fmt", &["fmt", "--all", "--check"]),
    (
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    ),
    ("test", &["test", "--workspace"]),
];

fn main() -> ExitCode {
    let task = env::args().nth(1).unwrap_or_default();
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    match task.as_str() {
        "ci" => run_gates(&cargo),
        "dist" => match dist::run(&cargo) {
            Ok(archive) => {
                let size = std::fs::metadata(&archive).map_or(0, |m| m.len());
                eprintln!(
                    "xtask: wrote {} ({:.1} MB)",
                    archive.display(),
                    size as f64 / 1e6
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("xtask: dist failed: {err}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask ci | cargo xtask dist");
            ExitCode::FAILURE
        }
    }
}

fn run_gates(cargo: &str) -> ExitCode {
    for (name, args) in GATES {
        eprintln!("xtask: {name}");
        match Command::new(cargo).args(*args).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("xtask: {name} failed ({status})");
                return ExitCode::FAILURE;
            }
            Err(err) => {
                eprintln!("xtask: could not run cargo {name}: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!("xtask: all gates green");
    ExitCode::SUCCESS
}
