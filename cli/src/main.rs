//! `tpt-verify` — CLI front end for the TPT Verify engine.
//!
//! A second distribution channel besides the browser/desktop paste flow:
//! this is what a pre-commit hook or CI job runs against changed `.rs`
//! files. Exit code doubles as the CI gate signal: `0` only when every
//! function in every given file analyzed clean; `1` if any function has a
//! finding or couldn't be analyzed (unsupported construct or a parse
//! error); `2` for a usage/IO error (bad args, unreadable file) — kept
//! distinct from `1` so a CI script can tell "your code has a real finding"
//! apart from "this run itself is broken".

use std::path::PathBuf;
use std::process::ExitCode;

use tpt_verify_engine::{analyze_rust_source, render_report};

fn main() -> ExitCode {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: tpt-verify <file.rs> [more.rs ...]");
        eprintln!();
        eprintln!("Analyzes each Rust source file for reachable division-by-zero and");
        eprintln!("assertion violations. Exits 0 if every function in every file analyzed");
        eprintln!("clean, 1 if any function has a finding or an unsupported construct, 2 on");
        eprintln!("a usage or file-read error.");
        return ExitCode::from(2);
    }

    let mut any_issue = false;
    let mut any_io_error = false;

    for path in &paths {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tpt-verify: couldn't read '{}': {e}", path.display());
                any_io_error = true;
                continue;
            }
        };

        let label = path.display().to_string();
        match analyze_rust_source(&source) {
            Ok(reports) => {
                let clean = reports.iter().all(|r| r.error.is_none() && r.result.as_ref().is_some_and(|res| res.is_clean()));
                if !clean {
                    any_issue = true;
                }
                println!("{}", render_report(&label, &reports));
            }
            Err(e) => {
                eprintln!("{label}: {e}");
                any_issue = true;
            }
        }
    }

    if any_io_error {
        ExitCode::from(2)
    } else if any_issue {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
