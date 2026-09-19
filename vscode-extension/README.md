# TPT Verify — VS Code extension

Inline diagnostics for reachable division-by-zero and violated `assert!`s
in Rust functions, right in the editor — shells out to the `tpt-verify`
CLI (`../cli/`) on save, rather than a separate "paste into a browser tab"
step. Not published to the Marketplace (that's a publishing step, out of
scope here) — install it locally as described below.

## What it does

- Runs `tpt-verify --json <file>` whenever you save a `.rs` file (or via
  the **TPT Verify: Analyze current file** command).
- Places a diagnostic on each analyzed function's declaration line for
  every finding: a warning for a division-by-zero/assertion finding, an
  informational note for a Pro range finding, and a hint for a function
  outside the supported subset (with the reason).
- Diagnostics land on the function's `fn name(...)` line, not the exact
  statement — the engine doesn't currently track source spans per
  finding, so this is an honest approximation, not a false claim of
  precision.

## Requirements

The `tpt-verify` CLI binary, built from this repo:

```sh
cargo build --release -p tpt-verify-cli            # free
cargo build --release -p tpt-verify-cli --features pro   # pro (adds --pdf, real interval checks)
```

## Local installation (no Marketplace)

1. Build the CLI (above) and note the binary's path
   (`target/release/tpt-verify` or `tpt-verify.exe` on Windows).
2. In this directory:
   ```sh
   npm install
   npm run compile
   npx vsce package
   ```
   This produces `tpt-verify-vscode-<version>.vsix`.
3. In VS Code: Extensions view → `...` menu → **Install from VSIX...** →
   select the file from step 2.
4. Open your Settings (`Ctrl+,`), search "TPT Verify", and set
   **Tpt Verify: Executable Path** to the binary path from step 1 (it
   defaults to `tpt-verify`, which only works if that's already on your
   `PATH`).
5. Open or save a `.rs` file with a function this tool can analyze (see
   the main repo README's "Supported subset") — diagnostics appear
   automatically.

## Settings

| Setting | Default | Description |
|---|---|---|
| `tptVerify.executablePath` | `tpt-verify` | Path to the CLI binary. |
| `tptVerify.runOnSave` | `true` | Re-run analysis automatically on save. |

## Development

```sh
npm install
npm run watch     # recompile on change
```

Press F5 in VS Code (with this folder open) to launch an Extension
Development Host with the extension loaded.
