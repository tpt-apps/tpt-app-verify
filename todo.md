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
- A branded PDF report layout instead of plain text (mirror fea-lite's
  dependency-free PDF writer in `engine/src/pdf.rs` if wanted).

## Phase 5 — Platform review follow-ups (2026-09-19)
> From a full read-through of `engine/`, `src/`, `desktop/`, `GUMROAD.md`.
> Full writeup with rationale for every item:
> `C:\Users\phill\.claude\plans\review-platform-for-bugs-sunny-kurzweil.md`

### Bugs
- [x] Unbounded `Box::leak` per variable name on every "Analyze" click
      (`engine/src/rust_frontend.rs`) — real memory leak in a long-lived
      SPA tab. Fix: intern distinct names once (bounded cache) instead of
      leaking fresh on every run; `"__return"` and other synthetic names
      don't need leaking at all (already `'static` literals).
- [x] Dead empty-`if` scaffolding in `rust_frontend.rs` (`analyze_fn`,
      the unused-return-type check) — remove or implement.
- [x] CI added (`.github/workflows/ci.yml`): runs `cargo test` in
      `engine/` (free + `--features pro`) on every push/PR — previously
      manual, and this is the correctness backbone for a paid
      formal-verification product.
- [ ] No README/LICENSE in the repo — legal ambiguity for a repo under
      `Open Source/`.
- [x] **Found during this pass, not in the original review**: a real bug
      in the shared `tpt-appfront` framework (`tpt-appfront-dom`'s
      `render_with`, used by every app on this framework, not just this
      one) silently dropped the `Err` `MountedRoot::render` returns on a
      nested node-kind mismatch, instead of the unmount+remount its own
      doc comment says callers "should" do — so *any* reactive state
      transition that changed a nested node's shape (e.g. this app's
      Results panel going from plain text to a findings list) silently
      failed to update the DOM, with no error, no panic, nothing in the
      console. This is why clicking "Analyze" appeared to do nothing
      before this fix. Fixed in
      `tpt-appfront/crates/tpt-appfront-dom/src/lib.rs`.

### Engine coverage
- [x] `if`/`else` support in the Rust-source frontend — the single
      biggest real-world coverage gap. Engine (`tpt-for-symbolic-exec`)
      already explores both branches correctly; this is frontend lowering
      only. v1 scope: both branches explored, div-by-zero/assert checks
      inside each; an if-expression's value can feed further arithmetic
      (via the store, `SExpr::Var`) but not a later `assert!`/`if`
      condition yet (that would need the condition to reason about a
      branch-dependent value, not just a free symbol — explicit error,
      not silently wrong, if attempted).
- [ ] Function calls to other analyzed functions (compositional
      verification) — still unsupported, real wall for factored code.

### GUI / usability
- [x] The shipped UI has zero custom styling (raw browser defaults) —
      add a real visual layout: header/brand, card-based sections, a
      "supported subset" cheat-sheet panel, and finding rows tagged
      exact-proof vs. best-effort inline (not just one global footer
      note).
- [x] In-product example gallery (was: exactly one hardcoded sample) —
      a small picker covering a guarded division, an unguarded one, and
      an if/else case now that it's supported.
- [ ] Line numbers / syntax highlighting in the paste textarea.
- [x] "Copy report to clipboard" next to "Download report" (Pro).

### Adoption
- [x] CLI edition (`cargo run -p tpt-verify-cli -- file.rs`) — a second
      distribution channel (CI/pre-commit gate) besides the browser/
      desktop paste flow.
- [x] Example GitHub Actions workflow showing the CLI wired into CI as a
      PR gate (`.github/workflows/`).
- [ ] VS Code extension (inline diagnostics) — the highest-leverage but
      largest item; deferred, not started.
- [ ] Permalink / shareable analysis (encode source+result in a URL).
- [ ] "Explain the proof" natural-language rendering of the witness path
      condition (currently raw AND'd boolean algebra).
- [ ] Diff mode (before/after paste, show only new/cleared findings).
