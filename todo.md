# TPT Verify — Build Todo

> Hub listing: `tptsolutions.co.nz/tools/verify` (registry slug `verify`,
> category `engineering`). Spec: `apps.md` §T1-3 in the `tpt-electrician-nz-2`
> repo. Price: $999 USD (Pro). Gumroad copy: `GUMROAD.md` in this repo.

## Phase 1 — Engine (`engine/`, pure Rust, host-testable)
- [x] Line-DSL front end (`parser.rs`): input/assign/assert/assume,
      arithmetic + comparisons, variable-definition inlining (required
      because `tpt-for-symbolic-exec`'s `Assert`/`Assume` conditions aren't
      evaluated through the store the way `Assign` right-hand sides are —
      see the module doc in `parser.rs`)
- [x] Rust-source front end (`rust_frontend.rs`, via `syn`): real `fn` items,
      integer params as inputs, `let` bindings, `assert!`/`debug_assert!`,
      `+ - * /` arithmetic, multi-function analysis in one pass
- [x] `assert!` lowers to both `Assert` (check) and `Assume` (narrow) — real
      Rust semantics guarantee the condition holds for everything after a
      passed assertion, which the underlying crate's `Assert` alone doesn't
      remember on its own
- [x] Tail expressions (implicit return, no `;`) are lowered into a
      synthetic `__return` assignment rather than discarded, so a division
      in a return value is still checked, not silently skipped
- [x] Every unsupported construct (loops, `if`, calls, non-integer params,
      structs/arrays, destructuring, `let else`) makes *that function*
      report an explicit error naming what and where — never silently
      ignored; other functions in the same paste are still analyzed
- [x] Pro feature: lightweight "unbounded input" range hint (flags free
      inputs with no `assert`/`assume` bounding their range) — a real
      CFG-based `tpt_for_abstract_interp::analyze` pass (per-statement
      interval tracking) is a stronger version of this, not yet wired in
- [x] Report export (`report.rs`, Pro): plain-text/Markdown summary across
      every analyzed function
- [x] 12/12 tests green in both `cargo test` and `cargo test --features pro`

## Phase 2 — Free WASM UI (tpt-appfront-dom)
- [x] Textarea for pasted Rust source + Analyze button, Elm-style
      (`Signal`-driven `view()`/`dispatch`, not manual DOM patching — simpler
      than fea-lite/cutlist-optimizer's approach since this UI has no
      canvas/complex reactivity needs)
- [x] Per-function results list: clean / N findings, tagged by kind
      (division-by-zero / assertion / range), with the witness path
      condition
- [x] Free-tier honesty note in the UI itself (not just marketing copy):
      explains the exact/best-effort boundary so a "clean" result on
      compound arithmetic isn't over-read as a proof
- [x] Pro: "Download report" button (Blob + object URL, no server round
      trip)
- [x] Browser-verified via the actual hub runner (`/tools/verify`): mounts,
      analyzes the default two-function sample, shows expected findings

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [x] `pro` gates: interval-based range hints, report export
- [x] Desktop shell via `tpt-appfront-webview` (same generic template as
      fea-lite/cutlist-optimizer); exe builds and launches without error
      (`target\release\tpt-verify-pro.exe`)
- [x] Packaged for Gumroad: `release\TPT Verify Pro\` (exe + dist) zipped to
      `release\TPT-Verify-Pro.zip`
- [ ] Interactive desktop smoke test in a real session (open exe, paste a
      function, analyze, download report) — built and launches without
      error, but not manually clicked through yet

## Phase 4 — Ship & measure
- [x] Registry entry live (`src/lib/tpt-apps-registry.ts`), free wasm bundle
      deployed to `public/apps/verify/` in the web repo
- [x] Verified in dev: `/tools/verify` (200), wasm/glue assets (200), hub
      card lists it
- [ ] Gumroad product from `GUMROAD.md`, URL + price into the registry entry
      — **manual, next step**
- [ ] Cover image/screenshot for the Gumroad listing

## Known engine limitations (real, not TODOs — see `GUMROAD.md`'s honest
scope note for the full explanation)
- No buffer-overflow/null-dereference detection: the engine is purely
  arithmetic; there's no memory/pointer abstract domain anywhere in
  `tpt-formal` to build that on. A real one is a research-scope project.
- Assertion checks on compound arithmetic (not a direct same-term guard)
  are best-effort, not a proof — the underlying `tpt-for-symbolic-exec` SMT
  backend is intentionally lightweight.

## Possible follow-ups (not blockers)
- Wire `tpt_for_abstract_interp::analyze`'s real fixpoint engine in for Pro
  range checking, replacing the current "unbounded input" heuristic.
- Support `if`/`else` bodies (the underlying crate's `SStmt::If` already
  exists; the Rust-frontend lowering for it doesn't yet).
- A branded PDF report layout instead of plain text (mirror fea-lite's
  dependency-free PDF writer in `engine/src/pdf.rs` if wanted).
