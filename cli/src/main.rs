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

mod json;

use std::path::PathBuf;
use std::process::ExitCode;

use tpt_verify_engine::{analyze_rust_source, render_report};
#[cfg(feature = "pro")]
use tpt_verify_engine::FunctionReport;

fn usage() {
    eprintln!("usage: tpt-verify [--json] [--pdf <output.pdf>] <file.rs> [more.rs ...]");
    eprintln!();
    eprintln!("Analyzes each Rust source file for reachable division-by-zero and");
    eprintln!("assertion violations (and, in the Pro build, real interval-based range");
    eprintln!("checking). Exits 0 if every function in every file analyzed clean, 1 if");
    eprintln!("any function has a finding or an unsupported construct, 2 on a usage or");
    eprintln!("file-read error.");
    eprintln!();
    eprintln!("--json prints one JSON object per file to stdout instead of the plain-text");
    eprintln!("report, for tooling (e.g. the VS Code extension) — see cli/src/json.rs for");
    eprintln!("the exact shape.");
    #[cfg(not(feature = "pro"))]
    eprintln!();
    #[cfg(not(feature = "pro"))]
    eprintln!("--pdf requires the Pro build (this is the free build).");
}

fn main() -> ExitCode {
    let mut pdf_out: Option<PathBuf> = None;
    let mut json_mode = false;
    let mut paths: Vec<PathBuf> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pdf" {
            let Some(out) = args.next() else {
                eprintln!("tpt-verify: --pdf needs a path argument");
                usage();
                return ExitCode::from(2);
            };
            pdf_out = Some(PathBuf::from(out));
        } else if arg == "--json" {
            json_mode = true;
        } else if arg == "-h" || arg == "--help" {
            usage();
            return ExitCode::SUCCESS;
        } else {
            paths.push(PathBuf::from(arg));
        }
    }

    if paths.is_empty() {
        usage();
        return ExitCode::from(2);
    }

    #[cfg(not(feature = "pro"))]
    if pdf_out.is_some() {
        eprintln!("tpt-verify: --pdf requires the Pro build (this is the free build)");
        return ExitCode::from(2);
    }

    let mut any_issue = false;
    let mut any_io_error = false;
    #[cfg(feature = "pro")]
    let mut all_reports: Vec<(String, Vec<FunctionReport>)> = Vec::new();
    let mut json_files: Vec<String> = Vec::new();

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
        let result = analyze_rust_source(&source);
        match &result {
            Ok(reports) => {
                let clean = reports.iter().all(|r| r.error.is_none() && r.result.as_ref().is_some_and(|res| res.is_clean()));
                if !clean {
                    any_issue = true;
                }
                if !json_mode {
                    println!("{}", render_report(&label, reports));
                }
                #[cfg(feature = "pro")]
                all_reports.push((label.clone(), reports.clone()));
            }
            Err(e) => {
                if !json_mode {
                    eprintln!("{label}: {e}");
                }
                any_issue = true;
            }
        }
        if json_mode {
            json_files.push(json::file_to_json(&label, &result));
        }
    }

    if json_mode {
        println!("{{\"files\":[{}]}}", json_files.join(","));
    }

    #[cfg(feature = "pro")]
    if let Some(out_path) = &pdf_out {
        // One combined report across every file passed, in the same order
        // as the text output above — simplest useful shape for "attach
        // this to the PR", which is the actual use case.
        let combined: Vec<FunctionReport> = all_reports.into_iter().flat_map(|(_, r)| r).collect();
        let label = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
        let bytes = tpt_verify_engine::render_report_pdf(&label, &combined);
        if let Err(e) = std::fs::write(out_path, bytes) {
            eprintln!("tpt-verify: couldn't write PDF to '{}': {e}", out_path.display());
            return ExitCode::from(2);
        }
        eprintln!("tpt-verify: wrote {}", out_path.display());
    }

    if any_io_error {
        ExitCode::from(2)
    } else if any_issue {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
