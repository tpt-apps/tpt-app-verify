//! Free WASM edition UI (tpt-appfront-dom), Elm-style: state lives in
//! `Signal`s, `view()` rebuilds the `UITree` from current state, and
//! `tpt_appfront_dom::render` reconciles the DOM on every signal change
//! `view()` reads.
//!
//! Input is real Rust source (parsed with `syn`), not a toy DSL — paste one
//! or more small functions and each is analyzed independently. See
//! `tpt_verify_engine::rust_frontend`'s module doc for the exact supported
//! subset; anything outside it is reported as an explicit per-function
//! error, never silently skipped.
//!
//! Styling follows the same hand-rolled-CSS-injected-at-mount pattern as
//! the sibling apps (fea-lite, cutlist-optimizer): a `BASE_CSS` string is
//! upserted into `document.head` once, and the mount root gets a
//! `verify-app` class every rule below is scoped under.

use std::rc::Rc;

use tpt_appfront_core::{ContainerBuilder, Signal, UITree};
use tpt_verify_engine::{analyze_rust_source, Finding, FindingKind, FunctionReport};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const DEFAULT_SOURCE: &str = SAMPLES[0].1;

/// Built-in examples, picked from the "how do I even try this" gap a
/// first-time visitor hits with only one hardcoded sample: each shows a
/// different real proof shape the engine now supports.
const SAMPLES: &[(&str, &str)] = &[
    (
        "Guarded vs. unguarded division",
        "// `checkout_total` divides by a free parameter with no guard, so\n\
         // TPT Verify should prove a reachable division by zero below. Guard it\n\
         // like `apply_discount` does and re-run to see that finding clear, while\n\
         // the precondition itself still gets checked separately.\n\
         \n\
         fn checkout_total(cents: i64, item_count: i64) -> i64 {\n    \
             cents / item_count\n\
         }\n\
         \n\
         fn apply_discount(cents: i64, divisor: i64) -> i64 {\n    \
             assert!(divisor != 0);\n    \
             cents / divisor\n\
         }\n",
    ),
    (
        "if/else — division on one path only",
        "// if/else is supported: both branches are explored independently.\n\
         // The 'tier > 0' branch guards its division; the 'else' branch divides\n\
         // by the same free parameter with no guard, so a reachable division by\n\
         // zero is proven on that path only — not the whole function.\n\
         \n\
         fn price_per_item(cents: i64, tier: i64, qty: i64) -> i64 {\n    \
             if tier > 0 {\n        \
                 assert!(qty != 0);\n        \
                 cents / qty\n    \
             } else {\n        \
                 cents / qty\n    \
             }\n\
         }\n",
    ),
    (
        "Precondition vs. consequence",
        "// The assert! is itself flagged as reachably violable (a caller really\n\
         // can pass divisor = 0) -- a real, useful finding. What it should NOT\n\
         // also flag is a division-by-zero on the next line: given the assert\n\
         // held, divisor != 0 is known for everything after it.\n\
         \n\
         fn charge(cents: i64, divisor: i64) -> i64 {\n    \
             assert!(divisor != 0);\n    \
             cents / divisor\n\
         }\n",
    ),
    (
        "Function calls (compositional)",
        "// Calls to other functions in this same paste are inlined and checked\n\
         // compositionally: 'checkout' passes a literal 0 into 'charge', so\n\
         // 'charge's OWN precondition is checked against that literal at the\n\
         // call site -- a real, exact violation, not just 'possibly'.\n\
         \n\
         fn charge(cents: i64, divisor: i64) -> i64 {\n    \
             assert!(divisor != 0);\n    \
             cents / divisor\n\
         }\n\
         \n\
         fn checkout(cents: i64) -> i64 {\n    \
             charge(cents, 0)\n\
         }\n",
    ),
    (
        "Compound assertion (best-effort)",
        "// Assertion-checking on compound conditions is best-effort, not a\n\
         // proof: this engine decides a DIRECT contradiction on the same value\n\
         // exactly, but 'x > 5' implying 'x > 0' is inequality-chaining, which\n\
         // the lightweight SMT backend doesn't decide. It reports this as\n\
         // 'possibly violated' rather than silently assuming it's safe -- the\n\
         // right direction to be wrong in -- but it's a prompt to look closer,\n\
         // not a confirmed bug.\n\
         \n\
         fn check(x: i64) -> i64 {\n    \
             assert!(x > 5);\n    \
             assert!(x > 0);\n    \
             x\n\
         }\n",
    ),
];

const BASE_CSS: &str = r#"
.verify-app{--vf-bg:#f7f7fb;--vf-surface:#ffffff;--vf-panel:#f1f1f8;--vf-fg:#191a23;--vf-muted:#65667a;--vf-border:#e3e3ee;--vf-accent:#5b45e0;--vf-accent-2:#4432b8;--vf-accent-soft:#ece9fb;--vf-accent-fg:#ffffff;--vf-err:#b3261e;--vf-err-bg:#fbeeec;--vf-warn:#8a5a00;--vf-warn-bg:#fbf3e2;--vf-info:#0a5f8a;--vf-info-bg:#e9f4fb;--vf-ok:#0e7c4a;--vf-ok-bg:#e9f7ef;--vf-shadow:0 1px 2px rgba(16,24,40,.05),0 1px 3px rgba(16,24,40,.06);max-width:960px;margin:0 auto;background:var(--vf-bg);color:var(--vf-fg);font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;font-size:14.5px;line-height:1.55;padding:0 4px 32px}
@media(prefers-color-scheme:dark){.verify-app{--vf-bg:#0c0d14;--vf-surface:#15161f;--vf-panel:#181a26;--vf-fg:#e9e9f2;--vf-muted:#9a9bb0;--vf-border:#292b3a;--vf-accent:#8a77f5;--vf-accent-2:#a597fa;--vf-accent-soft:#211d3b;--vf-accent-fg:#0b0714;--vf-err:#f2b8b5;--vf-err-bg:#2c1a1c;--vf-warn:#f0d090;--vf-warn-bg:#332912;--vf-info:#8fd0f0;--vf-info-bg:#132430;--vf-ok:#8fe0b8;--vf-ok-bg:#0f2b1e;--vf-shadow:0 1px 2px rgba(0,0,0,.3)}}
.verify-app *{box-sizing:border-box}
.verify-app h1{font-size:1.5rem;margin:0;font-weight:800;letter-spacing:-.01em}
.verify-app h2{font-size:1.05rem;margin:0 0 6px;font-weight:700}

.verify-header{display:flex;align-items:flex-start;gap:14px;padding:22px 2px 18px;border-bottom:1px solid var(--vf-border);margin-bottom:18px}
.verify-brand{flex:0 0 auto;width:40px;height:40px;border-radius:10px;background:linear-gradient(135deg,var(--vf-accent),var(--vf-accent-2));display:flex;align-items:center;justify-content:center;color:var(--vf-accent-fg);font-weight:800;font-size:.78rem;box-shadow:var(--vf-shadow)}

.verify-card{background:var(--vf-surface);border:1px solid var(--vf-border);border-radius:14px;padding:16px 18px;box-shadow:var(--vf-shadow);margin-bottom:16px}
.verify-hint{color:var(--vf-muted);font-size:.88rem;margin:0 0 12px}

.verify-samples{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:12px}
.verify-app button.verify-sample-btn{padding:6px 13px;font-size:.8rem;font-weight:600;border-radius:999px;border:1px solid var(--vf-border);background:var(--vf-panel);color:var(--vf-fg)}
.verify-app button.verify-sample-btn:hover{border-color:var(--vf-accent);color:var(--vf-accent)}

.verify-app textarea{width:100%;min-height:220px;padding:12px 14px;border:1px solid var(--vf-border);border-radius:10px;background:var(--vf-bg);color:var(--vf-fg);font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.86rem;line-height:1.5;resize:vertical;white-space:pre;overflow-x:auto}
.verify-app textarea:focus{outline:none;border-color:var(--vf-accent);box-shadow:0 0 0 3px var(--vf-accent-soft)}

.verify-editor{display:flex;align-items:stretch}
.verify-gutter{flex:0 0 auto;width:2.6em;padding:12px 6px 12px 0;border:1px solid var(--vf-border);border-right:none;border-radius:10px 0 0 10px;background:var(--vf-panel);color:var(--vf-muted);font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.86rem;line-height:1.5;text-align:right;white-space:pre;overflow:hidden;user-select:none}
.verify-editor textarea{border-radius:0 10px 10px 0;min-height:220px}

.verify-actions{display:flex;gap:8px;flex-wrap:wrap;margin-top:12px}
.verify-app button{padding:9px 18px;border-radius:9px;border:1px solid var(--vf-border);background:var(--vf-surface);color:var(--vf-fg);font:inherit;font-weight:700;cursor:pointer}
.verify-app button:hover{border-color:var(--vf-accent);color:var(--vf-accent)}
.verify-app button.verify-btn-primary{background:linear-gradient(135deg,var(--vf-accent),var(--vf-accent-2));border-color:transparent;color:var(--vf-accent-fg)}
.verify-app button.verify-btn-primary:hover{filter:brightness(1.08);color:var(--vf-accent-fg)}

.verify-subset{font-size:.82rem;color:var(--vf-muted)}
.verify-subset-row{display:flex;align-items:center;gap:8px;margin:4px 0}
.verify-subset-tag{flex:0 0 auto;font-weight:800;font-size:.68rem;text-transform:uppercase;letter-spacing:.03em;padding:2px 8px;border-radius:999px}
.verify-subset-ok{background:var(--vf-ok-bg);color:var(--vf-ok)}
.verify-subset-no{background:var(--vf-err-bg);color:var(--vf-err)}

.verify-result-line{padding:10px 0;border-bottom:1px dashed var(--vf-border)}
.verify-result-line:last-child{border-bottom:none}
.verify-finding-row{margin-top:6px}
.verify-badge{display:inline-block;font-size:.68rem;font-weight:800;text-transform:uppercase;letter-spacing:.03em;padding:2px 8px;border-radius:999px;margin-right:6px}
.verify-badge-div{background:var(--vf-err-bg);color:var(--vf-err)}
.verify-badge-assert{background:var(--vf-warn-bg);color:var(--vf-warn)}
.verify-badge-range{background:var(--vf-info-bg);color:var(--vf-info)}
.verify-badge-clean{background:var(--vf-ok-bg);color:var(--vf-ok);margin-right:8px}
.verify-finding-explain{color:var(--vf-fg);font-size:.83rem;font-style:italic;margin:3px 0 0}
.verify-finding-note{color:var(--vf-muted);font-size:.8rem;margin:2px 0 0}

.verify-diff-header{display:flex;align-items:center;justify-content:space-between;gap:12px}
.verify-diff-header h2{margin:0}
.verify-diff-cols{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:10px}
.verify-diff-cols h2{font-size:.8rem;text-transform:uppercase;letter-spacing:.04em;color:var(--vf-muted);margin:0 0 4px}
.verify-diff-new-header,.verify-diff-cleared-header{font-weight:700;font-size:.85rem;margin:12px 0 4px}
.verify-diff-cleared-header{color:var(--vf-ok)}

.verify-footer{margin-top:6px;color:var(--vf-muted);font-size:.82rem}
"#;

#[derive(Clone)]
enum Msg {
    SourceChanged(String),
    LoadSample(usize),
    Analyze,
    Share,
    ToggleDiff,
    BeforeChanged(String),
    AfterChanged(String),
    Compare,
    #[cfg(feature = "pro")]
    DownloadReport,
    #[cfg(feature = "pro")]
    DownloadPdf,
    #[cfg(feature = "pro")]
    CopyReport,
}

/// Default pair for the diff-mode card: an unguarded division whose finding
/// clears once guarded, while the guard itself introduces a new (real,
/// worth-a-look) finding — a single default example that demonstrates both
/// "new" and "cleared" without the visitor having to type anything first.
const DIFF_BEFORE: &str = "fn checkout_total(cents: i64, item_count: i64) -> i64 {\n    cents / item_count\n}\n";
const DIFF_AFTER: &str = "fn checkout_total(cents: i64, item_count: i64) -> i64 {\n    assert!(item_count != 0);\n    cents / item_count\n}\n";

enum DiffOutcome {
    Error(String),
    Ready { new_findings: Vec<(String, Finding)>, cleared_findings: Vec<(String, Finding)> },
}

/// Findings present in `after` but not `before` (by function name), and
/// vice versa. Matches functions by name; a renamed/added/removed function
/// counts all of its findings as entirely new/cleared. Two findings are
/// "the same" if their kind/summary/detail match — `explanation` is a pure
/// function of `detail`, so it never needs its own comparison.
fn diff_findings(before: &[FunctionReport], after: &[FunctionReport]) -> (Vec<(String, Finding)>, Vec<(String, Finding)>) {
    fn findings_of<'a>(reports: &'a [FunctionReport], name: &str) -> &'a [Finding] {
        reports
            .iter()
            .find(|r| r.name == name)
            .and_then(|r| r.result.as_ref())
            .map(|r| r.findings.as_slice())
            .unwrap_or(&[])
    }
    fn same(a: &Finding, b: &Finding) -> bool {
        a.kind == b.kind && a.summary == b.summary && a.detail == b.detail
    }

    let mut new_findings = Vec::new();
    for a in after {
        let Some(result) = &a.result else { continue };
        let before_findings = findings_of(before, &a.name);
        for f in &result.findings {
            if !before_findings.iter().any(|bf| same(bf, f)) {
                new_findings.push((a.name.clone(), f.clone()));
            }
        }
    }

    let mut cleared_findings = Vec::new();
    for b in before {
        let Some(result) = &b.result else { continue };
        let after_findings = findings_of(after, &b.name);
        for f in &result.findings {
            if !after_findings.iter().any(|af| same(af, f)) {
                cleared_findings.push((b.name.clone(), f.clone()));
            }
        }
    }

    (new_findings, cleared_findings)
}

struct RunReports {
    reports: Vec<FunctionReport>,
}

#[derive(Clone)]
enum Outcome {
    Idle,
    ParseError(String),
    Analyzed(Rc<RunReports>),
}

pub fn mount_app(container: &web_sys::Element) -> Result<(), JsValue> {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        upsert_style(&document, "tpt-verify-styles", BASE_CSS);
    }
    add_class(container, "verify-app");

    // A permalink restores the pasted source (not the analysis result —
    // recomputing is cheap and always fresh) from `#src=<percent-encoded>`
    // in the URL fragment, then auto-analyzes so a shared link reproduces
    // the finding immediately instead of landing on an empty "click
    // Analyze" state.
    let restored_source = read_source_from_hash();
    let starts_analyzed = restored_source.is_some();
    let source = Signal::new(restored_source.unwrap_or_else(|| DEFAULT_SOURCE.to_string()));
    let outcome = Signal::new(Outcome::Idle);
    if starts_analyzed {
        let next = match analyze_rust_source(&source.get()) {
            Ok(reports) => Outcome::Analyzed(Rc::new(RunReports { reports })),
            Err(e) => Outcome::ParseError(e),
        };
        outcome.set(next);
    }

    let diff_open = Signal::new(false);
    let diff_before = Signal::new(DIFF_BEFORE.to_string());
    let diff_after = Signal::new(DIFF_AFTER.to_string());
    let diff_result: Signal<Option<Rc<DiffOutcome>>> = Signal::new(None);

    let view: Rc<dyn Fn() -> UITree<Msg>> = {
        let source = source.clone();
        let outcome = outcome.clone();
        let diff_open = diff_open.clone();
        let diff_before = diff_before.clone();
        let diff_after = diff_after.clone();
        let diff_result = diff_result.clone();
        Rc::new(move || build_view(&source, &outcome, &diff_open, &diff_before, &diff_after, &diff_result))
    };

    let dispatch: Rc<dyn Fn(Msg)> = {
        let source = source.clone();
        let outcome = outcome.clone();
        let diff_open = diff_open.clone();
        let diff_before = diff_before.clone();
        let diff_after = diff_after.clone();
        let diff_result = diff_result.clone();
        Rc::new(move |msg| match msg {
            Msg::SourceChanged(s) => source.set(s),
            Msg::LoadSample(i) => {
                if let Some((_, src)) = SAMPLES.get(i) {
                    source.set(src.to_string());
                    outcome.set(Outcome::Idle);
                }
            }
            Msg::ToggleDiff => diff_open.set(!diff_open.get()),
            Msg::BeforeChanged(s) => diff_before.set(s),
            Msg::AfterChanged(s) => diff_after.set(s),
            Msg::Compare => {
                let next = match (analyze_rust_source(&diff_before.get()), analyze_rust_source(&diff_after.get())) {
                    (Ok(b), Ok(a)) => {
                        let (new_findings, cleared_findings) = diff_findings(&b, &a);
                        DiffOutcome::Ready { new_findings, cleared_findings }
                    }
                    (Err(e), _) => DiffOutcome::Error(format!("\"Before\" didn't parse: {e}")),
                    (_, Err(e)) => DiffOutcome::Error(format!("\"After\" didn't parse: {e}")),
                };
                diff_result.set(Some(Rc::new(next)));
            }
            Msg::Analyze => {
                let src = source.get();
                let next = match analyze_rust_source(&src) {
                    Ok(reports) => Outcome::Analyzed(Rc::new(RunReports { reports })),
                    Err(e) => Outcome::ParseError(e),
                };
                outcome.set(next);
            }
            Msg::Share => {
                let link = share_link(&source.get());
                copy_to_clipboard(&link);
            }
            #[cfg(feature = "pro")]
            Msg::DownloadReport => {
                if let Outcome::Analyzed(run) = outcome.get() {
                    let text = tpt_verify_engine::render_report("pasted source", &run.reports);
                    let _ = trigger_download(&text, "tpt-verify-report.txt");
                }
            }
            #[cfg(feature = "pro")]
            Msg::DownloadPdf => {
                if let Outcome::Analyzed(run) = outcome.get() {
                    let bytes = tpt_verify_engine::render_report_pdf("pasted source", &run.reports);
                    let _ = trigger_download_bytes(&bytes, "tpt-verify-report.pdf", "application/pdf");
                }
            }
            #[cfg(feature = "pro")]
            Msg::CopyReport => {
                if let Outcome::Analyzed(run) = outcome.get() {
                    let text = tpt_verify_engine::render_report("pasted source", &run.reports);
                    copy_to_clipboard(&text);
                }
            }
        })
    };

    let handle = tpt_appfront_dom::render(container, view, dispatch)?;
    // Whole-process root mount: forgetting is an explicit choice here, not
    // the default — this handle must live as long as the tab does.
    std::mem::forget(handle);

    // The line-number gutter is a read-only display, not a `tpt_appfront_dom`
    // node with its own event bindings, so its scroll position is synced to
    // the textarea's via one raw DOM `scroll` listener attached once here —
    // the textarea node itself is reused in place across every re-render
    // (only its content/attributes change), so this listener stays valid
    // for the app's whole lifetime without needing to be re-attached.
    wire_gutter_scroll_sync(container);

    Ok(())
}

fn wire_gutter_scroll_sync(container: &web_sys::Element) {
    let Some(textarea) = container.query_selector(".verify-editor textarea").ok().flatten() else { return };
    let Some(gutter) = container.query_selector(".verify-gutter").ok().flatten() else { return };
    let textarea_for_closure = textarea.clone();
    let closure = Closure::<dyn FnMut()>::new(move || {
        if let Some(ta) = textarea_for_closure.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            gutter.set_scroll_top(ta.scroll_top());
        }
    });
    let _ = textarea.add_event_listener_with_callback("scroll", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn build_view(
    source: &Signal<String>,
    outcome: &Signal<Outcome>,
    diff_open: &Signal<bool>,
    diff_before: &Signal<String>,
    diff_after: &Signal<String>,
    diff_result: &Signal<Option<Rc<DiffOutcome>>>,
) -> UITree<Msg> {
    UITree::container(|c| {
        c.container(|h| {
            h.container(|b| {
                b.text("TV");
            })
            .class("verify-brand");
            h.container(|t| {
                t.heading(1, "TPT Verify");
                hint(t, "Prove division-by-zero freedom and assertion reachability over real Rust functions — entirely in your browser, nothing uploaded.");
            });
        })
        .class("verify-header");

        c.container(|card| {
            card.heading(2, "Try an example");
            card.container(|row| {
                for (i, (label, _)) in SAMPLES.iter().enumerate() {
                    row.button(*label).class("verify-sample-btn").on_click(Msg::LoadSample(i));
                }
            })
            .class("verify-samples");
            card.container(|editor| {
                let line_count = source.get().lines().count().max(1);
                editor.container(|gutter| {
                    let numbers: String = (1..=line_count).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
                    gutter.text(numbers);
                })
                .class("verify-gutter")
                .attr("aria-hidden", "true");
                editor
                    .textarea(source.get())
                    .on_input(Msg::SourceChanged)
                    .attr("wrap", "off")
                    .attr("spellcheck", "false");
            })
            .class("verify-editor");
            card.container(|row| {
                row.button("Analyze").class("verify-btn-primary").on_click(Msg::Analyze);
                row.button("Share").on_click(Msg::Share);
                #[cfg(feature = "pro")]
                {
                    if matches!(outcome.get(), Outcome::Analyzed(_)) {
                        row.button("Download report").on_click(Msg::DownloadReport);
                        row.button("Download PDF").on_click(Msg::DownloadPdf);
                        row.button("Copy report").on_click(Msg::CopyReport);
                    }
                }
            })
            .class("verify-actions");
        })
        .class("verify-card");

        c.container(|card| {
            card.heading(2, "Results");
            match outcome.get() {
                Outcome::Idle => {
                    hint(card, "No analysis run yet — click Analyze, or load an example above.");
                }
                Outcome::ParseError(err) => {
                    hint(card, &format!("Couldn't parse this as Rust: {err}"));
                }
                Outcome::Analyzed(run) => render_reports(card, &run.reports),
            }
        })
        .class("verify-card");

        c.container(|card| {
            card.container(|row| {
                row.heading(2, "Diff mode");
                row.button(if diff_open.get() { "Hide" } else { "Compare before / after" }).on_click(Msg::ToggleDiff);
            })
            .class("verify-diff-header");
            if diff_open.get() {
                hint(card, "Paste a \"before\" and an \"after\" version of the same function(s) and see only what changed — findings that are new, and findings that cleared.");
                card.container(|cols| {
                    cols.container(|col| {
                        col.heading(2, "Before");
                        col.textarea(diff_before.get()).on_input(Msg::BeforeChanged);
                    });
                    cols.container(|col| {
                        col.heading(2, "After");
                        col.textarea(diff_after.get()).on_input(Msg::AfterChanged);
                    });
                })
                .class("verify-diff-cols");
                card.container(|row| {
                    row.button("Compare").class("verify-btn-primary").on_click(Msg::Compare);
                })
                .class("verify-actions");
                match diff_result.get() {
                    None => hint(card, "No comparison run yet — click Compare."),
                    Some(result) => render_diff(card, &result),
                }
            }
        })
        .class("verify-card");

        c.container(|card| {
            card.heading(2, "Supported subset");
            subset_row(card, true, "Integer parameters, 'let' bindings, +-*/, unary '-'");
            subset_row(card, true, "'assert!'/'debug_assert!', comparisons, && || !");
            subset_row(card, true, "'if'/'else' and 'else if' — both branches are explored");
            subset_row(card, true, "Calling other functions in this paste (inlined; no recursion)");
            subset_row(card, false, "Loops, recursion, non-integer types, structs/arrays, external calls");
            hint(
                card,
                "Anything outside this subset makes that function report an explicit \
                 \"not analyzed\" reason — it's never silently skipped.",
            );
        })
        .class("verify-card verify-subset");

        c.container(|footer| {
            footer.text(
                "Division-by-zero freedom is proven exactly when it follows a matching guard on \
                 the same value. Assertion reachability on compound arithmetic beyond a direct \
                 guard is best-effort: a flagged assertion is worth a manual look, not an \
                 automatic confirmed bug, and an unflagged compound assertion isn't yet a formal \
                 proof.",
            );
        })
        .class("verify-footer");
    })
}

/// A `.class()` call on a bare `text()` node is a silent no-op — the DOM
/// backend renders `text()` as a raw `Text` node (`document.create_text_node`),
/// which has no attributes to set a class on. Anywhere text needs styling,
/// it has to be wrapped in a `container()` (renders as a `<div>`) instead.
fn hint(c: &mut ContainerBuilder<Msg>, text: &str) {
    c.container(|w| {
        w.text(text.to_string());
    })
    .class("verify-hint");
}

fn tag(c: &mut ContainerBuilder<Msg>, label: &str, class: &str) {
    c.container(|w| {
        w.text(label.to_string());
    })
    .class(format!("verify-badge {class}"));
}

fn subset_row(c: &mut ContainerBuilder<Msg>, supported: bool, text: &str) {
    c.container(|row| {
        row.container(|w| {
            w.text(if supported { "works" } else { "not yet" });
        })
        .class(if supported { "verify-subset-tag verify-subset-ok" } else { "verify-subset-tag verify-subset-no" });
        row.text(text.to_string());
    })
    .class("verify-subset-row");
}

fn render_diff(c: &mut ContainerBuilder<Msg>, result: &DiffOutcome) {
    match result {
        DiffOutcome::Error(msg) => hint(c, msg),
        DiffOutcome::Ready { new_findings, cleared_findings } => {
            if new_findings.is_empty() && cleared_findings.is_empty() {
                hint(c, "No change in findings between \"before\" and \"after\".");
                return;
            }
            if !new_findings.is_empty() {
                c.container(|w| {
                    w.text(format!("{} new finding(s):", new_findings.len()));
                })
                .class("verify-diff-new-header");
                for (fn_name, finding) in new_findings {
                    diff_finding_row(c, fn_name, finding, "verify-badge-div");
                }
            }
            if !cleared_findings.is_empty() {
                c.container(|w| {
                    w.text(format!("{} finding(s) cleared:", cleared_findings.len()));
                })
                .class("verify-diff-cleared-header");
                for (fn_name, finding) in cleared_findings {
                    diff_finding_row(c, fn_name, finding, "verify-badge-clean");
                }
            }
        }
    }
}

fn diff_finding_row(c: &mut ContainerBuilder<Msg>, fn_name: &str, finding: &Finding, badge_class: &str) {
    c.container(|row| {
        tag(row, &format!("{:?}", finding.kind), badge_class);
        row.text(format!("fn {fn_name} — {}", finding.summary));
        row.container(|e| {
            e.text(finding.explanation.clone());
        })
        .class("verify-finding-explain");
    })
    .class("verify-finding-row");
}

fn render_reports(c: &mut ContainerBuilder<Msg>, reports: &[FunctionReport]) {
    // A plain `container()` (`<div>`), not `list()` (`<ul>`): the Results
    // card's other two states (`Outcome::Idle`/`ParseError`, via `hint()`)
    // also render as a `container()` at this exact tree position, so every
    // `Outcome` variant keeps the reconciled node's DOM tag identical
    // (`<div>`) across a state transition — swapping to a `<ul>` here left
    // this specific transition (Idle/ParseError -> Analyzed) silently
    // failing to update the DOM in testing (a `tpt_appfront_dom`
    // reconciliation edge case with nested, non-root tag-kind changes).
    c.container(|l| {
        for r in reports {
            l.container(|row| {
                if let Some(err) = &r.error {
                    row.text(format!("fn {}({}) — NOT ANALYZED: {}", r.name, r.params.join(", "), err));
                    return;
                }
                let Some(result) = &r.result else { return };
                if let Some(parse_err) = &result.parse_error {
                    row.text(format!("fn {}({}) — parse error: {}", r.name, r.params.join(", "), parse_err));
                    return;
                }
                if result.findings.is_empty() {
                    tag(row, "clean", "verify-badge-clean");
                    row.text(format!(
                        "fn {}({}) — clean across {} feasible path(s).",
                        r.name,
                        r.params.join(", "),
                        result.paths_explored
                    ));
                } else {
                    row.text(format!(
                        "fn {}({}) — {} finding(s):",
                        r.name,
                        r.params.join(", "),
                        result.findings.len()
                    ));
                    for finding in &result.findings {
                        let (label, class, note) = match finding.kind {
                            FindingKind::DivByZero => (
                                "Division by zero",
                                "verify-badge-div",
                                "Exact when this denominator is guarded by a direct comparison on the same value.",
                            ),
                            FindingKind::Assertion => (
                                "Assertion",
                                "verify-badge-assert",
                                "Best-effort on compound conditions — a real reachable panic if exact, otherwise worth a manual look.",
                            ),
                            FindingKind::RangeOverflow => (
                                "Range",
                                "verify-badge-range",
                                "Real interval analysis (fixpoint propagation through every assert/assume/if) — flags when the proven range of an arithmetic result actually touches i64::MIN/MAX, not just \"an input is unbounded\".",
                            ),
                        };
                        row.container(|f| {
                            tag(f, label, class);
                            f.text(format!("{} — {}", finding.summary, finding.detail));
                            f.container(|e| {
                                e.text(finding.explanation.clone());
                            })
                            .class("verify-finding-explain");
                            f.container(|n| {
                                n.text(note.to_string());
                            })
                            .class("verify-finding-note");
                        })
                        .class("verify-finding-row");
                    }
                }
            })
            .class("verify-result-line");
        }
    });
}

/// Creates or updates the `<style id>` in the document head.
fn upsert_style(document: &web_sys::Document, id: &str, css: &str) {
    if let Ok(Some(existing)) = document.query_selector(&format!("style#{id}")) {
        existing.set_text_content(Some(css));
        return;
    }
    let Some(head) = document.head() else { return };
    if let Ok(style_el) = document.create_element("style") {
        let _ = style_el.set_attribute("id", id);
        style_el.set_text_content(Some(css));
        let _ = head.append_child(&style_el);
    }
}

/// Adds `class` to `el` without disturbing whatever class the host page
/// (or the hub runner's container div) already put there.
fn add_class(el: &web_sys::Element, class: &str) {
    let existing = el.get_attribute("class").unwrap_or_default();
    if existing.split_whitespace().any(|c| c == class) {
        return;
    }
    let combined = if existing.is_empty() { class.to_string() } else { format!("{existing} {class}") };
    let _ = el.set_attribute("class", &combined);
}

#[cfg(feature = "pro")]
fn trigger_download(contents: &str, filename: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(contents));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/plain");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;

    let anchor = document.create_element("a")?.dyn_into::<web_sys::HtmlAnchorElement>()?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();

    web_sys::Url::revoke_object_url(&url)?;
    Ok(())
}

/// Same as `trigger_download`, but for binary content (the PDF report) —
/// `Blob`'s string constructor would corrupt non-UTF-8/binary bytes, so
/// this goes through a `Uint8Array` instead.
#[cfg(feature = "pro")]
fn trigger_download_bytes(bytes: &[u8], filename: &str, mime: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;

    let anchor = document.create_element("a")?.dyn_into::<web_sys::HtmlAnchorElement>()?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();

    web_sys::Url::revoke_object_url(&url)?;
    Ok(())
}

/// Best-effort clipboard copy — silently does nothing if the browser denies
/// clipboard access (e.g. an insecure context or missing permission). Used
/// by both the free "Share" link and (Pro) "Copy report"; a Pro user still
/// has "Download report"/"Download PDF" as reliable fallbacks if this
/// silently no-ops.
fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

/// Builds a shareable URL for `source`: the current page's URL (without any
/// existing hash) plus `#src=<percent-encoded source>`. Percent-encoding
/// (not base64) is deliberate — URL fragments already accept UTF-8 via
/// percent-escapes natively, so there's no `btoa`-is-Latin1-only pitfall to
/// work around for a comment containing e.g. an em dash.
fn share_link(source: &str) -> String {
    let href = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
    let base = href.split('#').next().unwrap_or(&href);
    let encoded = js_sys::encode_uri_component(source);
    format!("{base}#src={encoded}")
}

/// Reads a percent-encoded source back out of `#src=...` in the current
/// page's URL fragment, if present — the other half of `share_link`.
fn read_source_from_hash() -> Option<String> {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok())?;
    let encoded = hash.strip_prefix("#src=")?;
    let decoded = js_sys::decode_uri_component(encoded).ok()?;
    decoded.as_string()
}
