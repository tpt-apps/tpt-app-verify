# Gumroad listing

Everything needed to create/update the Gumroad product. Source copy is
`apps.md` §T1-3 in the `tpt-electrician-nz-2` repo — if that changes, update
here too (and vice versa).

## Step-by-step: publish this listing

1. Built — `release\TPT-Verify-Pro.zip` is ready to upload as-is. (Re-run
   `./build.ps1 -Pro`, `trunk build --release --features pro`, and
   `cargo build --release -p tpt-verify-desktop`, then re-stage/re-zip, before
   every price/content change to the Pro build.)
2. Log in to Gumroad → **New product** → **Digital product**.
3. Set the product name, price, and upload the packaged file (all below).
4. Paste the short description, bullets, and full description (below) into
   their respective fields.
5. Set category/tags (below) and add a cover image (see note below).
6. Publish, then copy the resulting product URL (`https://gum.co/...`).
7. Paste that URL into the `verify` entry's `gumroadUrl` field in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$999'`
   on the same entry.
8. Confirm `status` is `'live'` (it already is — the free demo shipped
   first).
9. Commit and push the registry repo.

## File to upload

```
release\TPT-Verify-Pro.zip
```

## Honest scope note (read before writing ad copy)

The engine parses **real Rust source** via `syn` (not a toy DSL) and
analyzes every `fn` in the pasted file independently. Free and Pro are
both fully tested (`cargo test` / `cargo test --features pro` in
`engine/`). Concretely, what's real:

- **Real Rust parsing.** Paste an actual function; integer parameters
  become inputs, `let` bindings, `assert!`/`debug_assert!`, and `+ - * /`
  arithmetic are understood directly from the Rust AST.
- **Multi-function analysis.** Every `fn` in the pasted source is analyzed
  and reported on independently in one pass.
- **`if`/`else` and `else if`** — both branches are proven independently
  (real path exploration, not a coverage skip). A finding on only one
  branch of an `if` is a real, common shape this now catches.
- **Calls to other functions in the same paste** are inlined and checked
  compositionally — a callee's own `assert!`s and divisions are checked
  *in the caller's context*, so passing a bad literal into a helper
  function is caught at the call site, not just inside the helper in
  isolation. Direct/mutual recursion is rejected explicitly (not silently
  wrong) since unbounded inlining has no termination guarantee.
- **Exact division-by-zero proofs**, including across a real precondition
  guard: `assert!(x != 0)` followed by `10 / x` is proven safe from
  division-by-zero (the assert's condition is known to hold for everything
  after it, matching real program semantics) — while the assert itself is
  *also* correctly flagged as reachably violable if a caller can actually
  pass `x = 0`. That distinction (this can panic vs. given it didn't panic,
  the division is safe) is the actual differentiator over a linter.
- **Real interval analysis (Pro)** — a genuine fixpoint solver
  (`tpt-for-abstract-interp`) propagates value ranges through every
  `assert`/`assume`/`if`, and flags an arithmetic expression only when its
  *actual proven range* touches `i64::MIN`/`MAX` — not a keyword-matching
  heuristic ("has this input got an assert on it at all").
  Automatically clears once you bound the inputs enough, and still catches
  a bound that's technically present but too wide.
- **Report export (Pro)** — plain-text/Markdown, copy-to-clipboard, and PDF
  (`engine/src/pdf.rs`, dependency-free, mirrors fea-lite's writer) across
  every analyzed function: status, findings, and a methodology note.
- **CLI edition** (`tpt-verify-cli`) with CI-friendly exit codes — a
  pre-commit/CI gate, not just a browser demo.
- **Never silently skips code it can't model.** Anything outside the
  supported subset (loops, non-integer types, structs, arrays/pointers,
  method calls, calls to external/library functions, recursion, ...)
  makes *that function* report an explicit "not analyzed: <reason>"
  rather than being ignored. Other functions in the same paste are still
  analyzed.

**What's still a real, stated boundary — don't oversell this:**
- **No buffer-overflow / null-dereference detection.** This engine is
  purely arithmetic (`tpt-for-symbolic-exec`'s model has no arrays, no
  pointers, no memory at all) — there is no memory-safety abstract domain
  anywhere in `tpt-formal` to build this on. Building one from scratch is a
  real research-scope project, not a checkbox — don't claim this capability
  until it's actually built.
- **Assertions on compound arithmetic are best-effort.** The underlying
  `tpt-for-symbolic-exec` SMT backend is intentionally lightweight ("minimal
  evaluator" per its own docs) and only decides direct same-term
  contradictions exactly — inequality chaining (`x > 5` implying `x > 0`)
  and reasoning about derived expressions aren't decided. The tool
  conservatively reports these as "possibly violated" rather than silently
  assuming safety (the correct direction to be wrong in), but a flagged
  compound assertion is a prompt to look closer, not an automatically
  confirmed defect.
- **No loops or recursion.** Both are rejected explicitly, not silently
  approximated — there's no widening-based loop analysis here (the
  interval analysis's own fixpoint engine has one internally, for its own
  narrow CFG, but the Rust-source frontend doesn't expose loops to it).

Net: market the Rust-source parsing, the multi-function and now
if/else + call-inlining coverage, the exact division-by-zero guarantee
(including the precondition-vs-consequence distinction), the real interval
analysis, and the CLI/CI story — all real and tested today. Don't market
blanket "finds memory-safety bugs" or "handles any Rust code" claims.

## Product name

```
TPT Verify — Formal Verification for Real Rust Functions
```

## Price

**$999 USD, one-time purchase** (no subscription). Restored to the original
target price — the v1 build now genuinely does real Rust-source parsing,
multi-function analysis, and report export, not just a narrow single-DSL
demo, so the original "biggest incumbent gap on the list" reasoning holds
(still 10–50x cheaper than Astrée/Polyspace for the capability it does have).

## Short description / summary field

```
Paste a real Rust function, get an exact proof it can't divide by zero on any reachable path — powered by real symbolic execution with an SMT backend, not a linter guessing from patterns.
```

## Bullets / feature list

```
Parses real Rust source (not a toy language)
Multi-function analysis in one pass
Supports if/else and else-if — both branches proven independently
Exact division-by-zero proofs, including precondition guards
Distinguishes "can panic" from "safe given it didn't"
Real interval analysis for overflow risk (Pro), not a keyword heuristic
Downloadable/copyable verification report (Pro)
CLI edition for CI/pre-commit gates
Runs fully offline, no upload
One-time purchase
```

## Full description (long-form field)

```
TPT Verify parses real Rust functions and runs actual symbolic execution
(with an SMT solver pruning infeasible paths) over them — proving, not
guessing, whether a division-by-zero is reachable. Guard an input with
assert!(x != 0) and the division after it is proven safe; leave it
unguarded and the tool proves the opposite. It also separately flags when a
precondition itself can be violated (a caller passing x = 0) — the
distinction between "this can panic" and "given it didn't panic, the rest
is safe" is the actual point of formal verification, not just pattern
matching.

Who it's for: engineers and teams who want a real, working formal-methods
check on the arithmetic-heavy parts of their code — calculations where a
silent division-by-zero or an unguarded precondition would be a real
incident, not just students exploring the idea.

Also ships a CLI (`tpt-verify file.rs`) with CI-friendly exit codes, so it
can gate a PR the same way a linter does, not just live in a browser tab.

Pro edition (this download):
- The full analysis engine, unlocked from the free browser demo's line/size
  limits
- Downloadable and copy-to-clipboard verification report across every
  function in your paste — status, findings, and methodology notes, ready
  to attach to a PR or a design review
- Real interval analysis (a fixpoint solver, not a keyword heuristic):
  proves whether an arithmetic expression's result can actually reach
  i64::MIN/MAX given everything your asserts/if-branches establish about
  its inputs, and clears the finding automatically once you bound them
  enough — not just "this input has no assert on it at all"
- PDF export of the verification report
- Yours to keep, runs fully offline forever, no subscription

Honest scope: this analyzes integer-arithmetic Rust functions — integer
params, let bindings, assert!/debug_assert!, + - * /, if/else and else-if
(both branches explored and proven independently), and calls to other
functions defined in the same paste (inlined and checked compositionally;
recursion is rejected explicitly). It does not yet detect memory-safety
issues like buffer overflows (no memory model exists for that in the
underlying engine), doesn't support loops or recursion, and
assertion-checking on compound arithmetic conditions is best-effort, not a
blanket proof. Division-by-zero freedom following a real guard is exact.

Try the free edition first, right in your browser, no install:
tptsolutions.co.nz/tools/verify

Requirements: Windows 10 (20H2+) or Windows 11. Uses the WebView2 runtime,
preinstalled on both — nothing extra to download.
```

## Category / tags

- Category: **Software > Developer Tools** (or Gumroad's closest equivalent)
- Tags: `formal-verification`, `symbolic-execution`, `smt-solver`,
  `static-analysis`, `rust`, `windows`

## Cover image / thumbnail

A candidate is at `assets/gumroad-cover-candidate.png` — the actual Pro
desktop exe, analyzed, showing one real division-by-zero finding and one
real assertion finding side by side (with badges, witness path, and the
plain-English explanation), captured during this session's interactive
desktop smoke test. Crop/resize to Gumroad's cover dimensions before
uploading; re-capture from a fresh run if the sample set changes. No
stock/AI art.

## After publishing

1. Copy the resulting product URL (`https://gum.co/...`).
2. Paste it into the `gumroadUrl` field of the `verify` entry in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$999'`.
3. Confirm `status` is `'live'`.
4. Commit and push the registry repo.
