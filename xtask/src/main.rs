//! `cargo xtask <task>` — developer tasks.
//!
//! - `ci`: all quality gates (fmt, clippy, test). Run before every commit.

#![forbid(unsafe_code)]

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
    match task.as_str() {
        "ci" => run_gates(),
        _ => {
            eprintln!("usage: cargo xtask ci");
            ExitCode::FAILURE
        }
    }
}

fn run_gates() -> ExitCode {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    for (name, args) in GATES {
        eprintln!("xtask: {name}");
        match Command::new(&cargo).args(*args).status() {
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
