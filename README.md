# tpt-app-verify

TPT Verify — formal verification for real Rust functions: proves reachable
division-by-zero and violated `assert!`s via real symbolic execution (an
SMT backend, not a linter guessing from patterns). Three editions ship
from this one repo:

- **Free WASM edition** — runs in-browser on
  `tptsolutions.co.nz/tools/verify` (registry slug `verify`, category
  `engineering`). Paste one or more Rust functions, get exact
  division-by-zero proofs, assertion-reachability checks, `if`/`else`
  path exploration, and compositional checking across calls to other
  functions in the same paste.
- **Pro desktop edition** ($999, Gumroad) — the same UI built with
  `--features pro`, which compiles in real interval-based range/overflow
  analysis (a genuine fixpoint solver, not a keyword heuristic), and
  downloadable/copyable/PDF report export, hosted in a native webview
  shell (`desktop/`) as a Windows exe.
- **CLI edition** (`cli/`, binary `tpt-verify`) — the same engine as a
  command-line tool with CI-friendly exit codes (`0` clean, `1` findings,
  `2` usage/IO error), for a pre-commit hook or a CI gate instead of a
  browser tab. `--features pro` adds `--pdf <path>`.

UI is built on [`tpt-appfront`](https://github.com/tpt-solutions/tpt-appfront)
(DOM backend); the verification engine lives in the UI-free `engine/`
crate, a `syn`-based Rust-source front end over the real formal-methods
workspace [`tpt-formal`](https://github.com/tpt-solutions/tpt-formal)
(`tpt-for-symbolic-exec` for division-by-zero/assertion checking,
`tpt-for-abstract-interp` for Pro's interval analysis). The engine is
pure Rust, host-testable (`cargo test`), and wasm-compatible without
changes.

## Layout

```
engine/                 pure Rust verification engine (the moat, host-testable)
  src/parser.rs         line-based DSL front end (input/assign/assert/assume)
  src/rust_frontend.rs  real Rust-source front end (via syn): fn items,
                        if/else, calls to other functions (inlined,
                        compositional), the exact supported subset is in
                        this file's module doc
  src/range.rs          (pro) real interval analysis wired to
                        tpt-for-abstract-interp's fixpoint engine
  src/report.rs         plain-text/Markdown report + (pro) PDF export
  src/pdf.rs            (pro) minimal PDF report writer (no external crates)
src/
  lib.rs                wasm-bindgen hub contract: `mount(el)` + `start()`
  app.rs                the DOM app: examples gallery, line-number gutter,
                        permalink sharing, diff mode, results/subset panels
desktop/                tpt-appfront-webview shell hosting the trunk bundle
cli/                    `tpt-verify` binary — CI/pre-commit entry point
vscode-extension/       inline diagnostics on save, shells out to the CLI
                        (not published to the Marketplace — see its own
                        README for local installation)
.github/workflows/      CI (engine tests, free + pro) and an example
                        CLI-as-PR-gate workflow (manual trigger — see the
                        workflow file for why it isn't a live gate on this
                        repo's own source)
build.ps1               wasm pipeline: cargo -> wasm-bindgen -> hub bundle,
                        or (`-Pro`) trunk + desktop exe
GUMROAD.md              Gumroad listing copy + the honest capability/scope
                        note (read before writing any marketing copy)
```

## Hub contract

`wasm-bindgen --target web` glue exposes the named export `mount(container)`
plus the `default` init export. The hub's `WasmAppRunner` dynamic-imports
the glue, awaits `default()`, then calls `mount(container)`. Standalone
`trunk serve` mounts into the `#tpt-appfront-root` marker div instead —
never `document.body` — so the two hosts never double-mount.

## Commands

```pwsh
trunk serve                          # standalone dev page, free edition
trunk build                          # free wasm bundle -> dist/
trunk build --features pro           # pro wasm bundle -> dist/ (desktop's source)
./build.ps1                          # free edition -> web repo (or dist\web)
./build.ps1 -Pro                     # pro wasm + trunk bundle + desktop exe

cargo test -p tpt-verify-engine                  # engine tests (free)
cargo test -p tpt-verify-engine --features pro   # engine tests (pro, incl. range.rs)

cargo build -p tpt-verify-cli                    # CLI (free)
cargo build -p tpt-verify-cli --features pro     # CLI (pro, adds --pdf)
./target/debug/tpt-verify file.rs [more.rs ...]  # analyze; see --help
```

Set `TPT_WEB_REPO` to the tptsolutions.co.nz web repo path and `build.ps1`
deploys the free bundle to `<repo>\public\apps\verify\` directly.

## Supported subset

See `engine/src/rust_frontend.rs`'s module doc for the exact, exhaustively
checked list (it's also shown live in the free/Pro UI's "Supported subset"
panel). In short: integer parameters, `let` bindings, `+ - * /`, unary
`-`, `assert!`/`debug_assert!`, comparisons, `&&`/`||`/`!`, `if`/`else`/
`else if` (both branches explored independently), and calls to other
functions in the same paste (inlined, checked compositionally, no
recursion). Anything outside that subset makes *that function* report an
explicit "not analyzed: `<reason>`" — never silently skipped; other
functions in the same paste are still analyzed.

## Engine notes

- Division-by-zero freedom is proven exactly when the divisor is guarded
  by a direct comparison on the same value (`assert!(x != 0)` then
  `10 / x`); the precondition itself is separately flagged as reachably
  violable if a caller can actually trigger it.
- Assertion-checking on compound arithmetic (not a direct same-term
  contradiction) is best-effort, not a blanket proof — the underlying
  `tpt-for-symbolic-exec` SMT backend is intentionally lightweight.
- (Pro) Range/overflow checking is real interval fixpoint analysis
  (`tpt-for-abstract-interp::analyze` over a CFG built from the Rust
  frontend's statement/branch tree), not a keyword heuristic — it flags
  an expression only when its *proven* range touches `i64::MIN`/`MAX`.
- No memory-safety detection (buffer overflows, null derefs): the engine
  is purely arithmetic, with no memory/pointer abstract domain anywhere
  in `tpt-formal` to build that on.
- No loops or recursion — both rejected explicitly, never silently
  approximated.

## Framework/dependency patches (upstreamed to their own repos)

Two real bugs in shared dependencies were found and fixed while building
this app's `if`/`else` and function-call-inlining support (both pushed
upstream, not vendored locally — this repo's `Cargo.toml`s pin the fixed
commits):

1. **`tpt-appfront-dom`** (`render_with`): a nested reconcile that hit a
   DOM node-kind mismatch (e.g. a `match` arm going from plain text to a
   `List`) returned `Err`, per `MountedRoot::render`'s own documented
   contract — but the effect loop silently discarded that `Err` instead
   of the unmount-and-remount the contract calls for, so the DOM could
   get stuck showing stale content with no error, panic, or console
   output anywhere. Fixed to actually follow its own contract — which
   then exposed a second, deeper bug in the same area:
   `MountedRoot::unmount` (and the router's equivalent,
   `reconcile_into_container`) tried to detach the mounted node from
   *its container's own parent* instead of the container itself (the
   node's actual parent, per how `mount` attaches it), so `remove_child`
   silently failed and the "old" tree was never actually removed —
   every kind-mismatch remount *appended* a second full copy of the UI
   next to the stale one instead of replacing it. This is what two
   already-existing but until-now-failing tests in that crate
   (`mount_then_unmount_removes_dom_and_listeners`,
   `conditional_subtree_swap_unmounts_old_listeners`) had been catching
   all along; both pass now, plus a new regression test exercising the
   exact remount-without-duplication path.
2. **`tpt-for-symbolic-exec`** (`exec`'s `Assign` handling): a division
   was re-detected and re-reported once per hop in any `x = y; z = x;`
   passthrough chain, because it checked the store-*substituted* value
   (which still contains the original `Div` node, since division isn't
   reduced) instead of the statement's own syntax. Both `if`/`else`
   branch-merged values and inlined function-call results lean heavily
   on exactly this pattern, so this would have shown 2-3x duplicate
   findings for a single real division. Fixed to check divisions where
   they're syntactically written, evaluating only the denominator
   through the store (needed for a genuinely new division whose
   denominator is itself a branch-merged value).
