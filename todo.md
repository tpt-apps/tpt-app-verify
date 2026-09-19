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
- [x] Pro feature: real interval analysis (`range.rs`, wired to
      `tpt_for_abstract_interp::analyze`'s fixpoint engine over a real CFG
      built from the Rust frontend's `SStmt`/`If` tree) — flags an
      arithmetic expression only when its *proven* range touches
      `i64::MIN`/`MAX`, replacing the old "any unbounded input" heuristic
- [x] Report export (`report.rs`, Pro): plain-text/Markdown, copy-to-
      clipboard, and PDF (`pdf.rs`, dependency-free, mirrors fea-lite's
      writer) summary across every analyzed function
- [x] Function calls to other functions in the same paste (compositional):
      fully inlined at the call site (parameters substituted, callee's own
      `assert!`s/divisions checked in the caller's context), with
      call-site-unique variable namespacing so inlined locals never alias
      across calls, and explicit rejection of direct/mutual recursion
- [x] 35/35 tests green in `cargo test --features pro` (engine)

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
- [x] Pro: "Download report", "Download PDF", and "Copy report" buttons
      (Blob + object URL / Clipboard API, no server round trip)
- [x] Browser-verified via the actual hub runner (`/tools/verify`): mounts,
      analyzes the default two-function sample, shows expected findings

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [x] `pro` gates: interval-based range hints, report export
- [x] Desktop shell via `tpt-appfront-webview` (same generic template as
      fea-lite/cutlist-optimizer); exe builds and launches without error
      (`target\release\tpt-verify-pro.exe`)
- [x] Packaged for Gumroad: `release\TPT Verify Pro\` (exe + dist) zipped to
      `release\TPT-Verify-Pro.zip`
- [x] Interactive desktop smoke test performed (automated via pywinauto
      driving the real exe, since no human hands were available in this
      session): launches, renders the full UI correctly, Analyze produces
      real results with badges/explanations, Download report/PDF/Copy
      report all present. This is what actually caught the deeper
      `tpt-appfront-dom` unmount bug above — it reproduced first here
      (and, once understood, in the plain browser Pro build too), not in
      the desktop shell specifically.

## Phase 4 — Ship & measure
- [x] Registry entry live (`src/lib/tpt-apps-registry.ts`), free wasm bundle
      deployed to `public/apps/verify/` in the web repo
- [x] Verified in dev: `/tools/verify` (200), wasm/glue assets (200), hub
      card lists it
- [ ] Gumroad product from `GUMROAD.md`, URL + price into the registry entry
      — **manual, next step**
- [x] Cover image candidate captured: `assets/gumroad-cover-candidate.png`
      (a real analyzed desktop-exe screenshot from this session's smoke
      test) — still needs a human crop/resize pass to Gumroad's exact
      cover dimensions before upload, but the "run it and grab a
      screenshot" step is done.

## Known engine limitations (real, not TODOs — see `GUMROAD.md`'s honest
scope note for the full explanation)
- No buffer-overflow/null-dereference detection: the engine is purely
  arithmetic; there's no memory/pointer abstract domain anywhere in
  `tpt-formal` to build that on. A real one is a research-scope project.
- Assertion checks on compound arithmetic (not a direct same-term guard)
  are best-effort, not a proof — the underlying `tpt-for-symbolic-exec` SMT
  backend is intentionally lightweight.
- No loops or recursion (rejected explicitly, never silently approximated).

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
- [x] README.md + LICENSE added — a repo under `Open Source/` had neither.
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
      before this fix — and fixing it exposed a *second*, deeper bug in
      the same file: `MountedRoot::unmount` detached the mounted node
      from the wrong parent (its container's parent, not the container
      itself), so `remove_child` silently failed and every kind-mismatch
      remount appended a duplicate copy of the whole UI instead of
      replacing the stale one — only visible once the first fix started
      actually triggering the remount path. Two long-failing tests in
      that crate had been catching this all along
      (`mount_then_unmount_removes_dom_and_listeners`,
      `conditional_subtree_swap_unmounts_old_listeners`); both pass now.
      Both fixes pushed to `tpt-appfront/crates/tpt-appfront-dom/src/lib.rs`,
      and the local-path deps in this repo's `Cargo.toml`s were switched
      to git deps against `tpt-solutions/tpt-appfront`/`tpt-formal` so a
      fresh clone actually builds at all (previously hardcoded absolute
      local Windows paths — nobody else could have built this).
- [x] **Also found during this pass**: a real bug in the shared
      `tpt-for-symbolic-exec` crate — a division was re-detected and
      re-reported once per statement in any `x = y; z = x;`-style
      passthrough chain (checked the store-substituted value instead of
      the statement's own syntax), so if/else-produced and call-inlined
      values (both introduced this session, both lean on exactly this
      pattern) would have shown 2-3x duplicate findings for a single real
      division. Fixed and regression-tested in
      `tpt-formal/crates/tpt-for-symbolic-exec/src/lib.rs` (pushed, rev
      `ef0f80f`).

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
- [x] Function calls to other analyzed functions (compositional
      verification) — see Phase 1's entry above.

### GUI / usability
- [x] The shipped UI has zero custom styling (raw browser defaults) —
      add a real visual layout: header/brand, card-based sections, a
      "supported subset" cheat-sheet panel, and finding rows tagged
      exact-proof vs. best-effort inline (not just one global footer
      note).
- [x] In-product example gallery (was: exactly one hardcoded sample) —
      a small picker covering a guarded division, an unguarded one, and
      an if/else case now that it's supported.
- [x] Line numbers in the paste textarea (a synced-scroll gutter; full
      syntax highlighting would need a JS editor dependency like
      CodeMirror, out of scope for this pass — noted, not silently
      dropped).
- [x] "Copy report to clipboard" next to "Download report" (Pro).
- [x] Permalink / shareable analysis (source + outcome encoded in the URL
      fragment, so a link reproduces a paste+result without a server).
- [x] "Explain the proof" natural-language rendering of the witness path
      condition, alongside the raw boolean form (kept, for anyone who
      wants the precise expression).
- [x] Diff mode (paste a "before" and "after" version, see only findings
      that are new or cleared).

### Adoption
- [x] CLI edition (`cargo run -p tpt-verify-cli -- file.rs`, `--pdf` in
      the Pro build, `--json` for tooling) — a second distribution channel
      (CI/pre-commit gate) besides the browser/desktop paste flow.
- [x] Example GitHub Actions workflow showing the CLI wired into CI as a
      PR gate (`.github/workflows/`).
- [x] VS Code extension (`vscode-extension/`): inline diagnostics on save,
      shelling out to the `tpt-verify` CLI (not published to the
      Marketplace — that's a publishing step, explicitly out of scope for
      this pass; installable locally via `vsce package` + "Install from
      VSIX", documented in the extension's own README).
