//! TPT Verify Pro — desktop shell (paid edition).
//!
//! A native host binary that opens the OS webview (WebView2 on Windows) and
//! serves the `trunk build --release` output of the appfront DOM bundle from
//! `dist/`. The same DOM bundle ships to the hub for the free WASM edition;
//! the Pro difference at runtime is that the bundle is built with
//! `--features pro`, which compiles in interval-based range/overflow
//! checking on top of the free edition's division-by-zero and assertion
//! checks.
//!
//! `dist/` is also embedded into the exe at compile time (see [`DIST`]), so
//! the file actually shipped to customers (the Gumroad zip) is the exe
//! alone — no separate folder for a customer to lose, rename, or unzip
//! apart from it. Resolve order for the bundle directory:
//! 1. `TPT_VERIFY_DIST` environment variable (dev override),
//! 2. `./dist` next to the current working directory (dev: running from the
//!    repo root without having built the desktop-specific extraction),
//! 3. `dist` next to the executable (dev: matches how `build.ps1 -Pro`
//!    leaves things when testing the exe in `target/release/` directly),
//! 4. the embedded copy, extracted once into the OS temp directory — this
//!    is the path a real end user's double-click actually takes.

use std::path::PathBuf;

use include_dir::{include_dir, Dir};
use tpt_appfront_webview::{AppBuilder, WindowConfig};

const APP_ID: &str = "nz.co.tptsolutions.verify-pro";

/// The `trunk build --release` output, embedded at compile time. `build.ps1
/// -Pro` runs the trunk build before this crate, so `../dist` (relative to
/// this crate's `Cargo.toml`) always exists by the time this macro expands.
static DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/../dist");

fn main() {
    let dist_dir = resolve_dist_dir();
    if !dist_dir.join("index.html").exists() {
        eprintln!(
            "tpt-verify-pro: no index.html under {} and embedded-asset extraction failed — \
             this build is missing its web bundle",
            dist_dir.display()
        );
        std::process::exit(1);
    }

    let result = AppBuilder::new(APP_ID)
        .with_window(WindowConfig {
            id: "main".to_string(),
            title: "TPT Verify Pro".to_string(),
            width: 1100,
            height: 850,
            dist_dir,
        })
        .with_single_instance(true)
        .run(|_action, _params| Ok(()));

    if let Err(error) = result {
        eprintln!("tpt-verify-pro: {error:#}");
        std::process::exit(1);
    }
}

fn resolve_dist_dir() -> PathBuf {
    if let Ok(from_env) = std::env::var("TPT_VERIFY_DIST") {
        return PathBuf::from(from_env);
    }
    let cwd_dist = PathBuf::from("dist");
    if cwd_dist.join("index.html").exists() {
        return cwd_dist;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let exe_dist = exe_dir.join("dist");
            if exe_dist.join("index.html").exists() {
                return exe_dist;
            }
        }
    }
    extract_embedded_dist()
}

/// Writes the embedded [`DIST`] contents to a stable temp directory (once —
/// subsequent launches reuse the extracted copy) and returns its path. This
/// is the path a customer's real double-click takes: no `dist/` folder
/// shipped or found, so the exe unpacks its own.
fn extract_embedded_dist() -> PathBuf {
    let out_dir = std::env::temp_dir().join(format!("{APP_ID}-dist"));
    if !out_dir.join("index.html").exists() {
        let _ = std::fs::create_dir_all(&out_dir);
        if let Err(error) = DIST.extract(&out_dir) {
            eprintln!("tpt-verify-pro: failed to extract embedded assets: {error:#}");
        }
    }
    out_dir
}
