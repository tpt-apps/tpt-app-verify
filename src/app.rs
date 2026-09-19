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

use std::rc::Rc;

use tpt_appfront_core::{ContainerBuilder, Signal, UITree};
use tpt_verify_engine::{analyze_rust_source, FindingKind, FunctionReport};
use wasm_bindgen::prelude::*;
#[cfg(feature = "pro")]
use wasm_bindgen::JsCast;

const DEFAULT_SOURCE: &str = "\
// Try it: `checkout_total` divides by a free parameter with no guard, so\n\
// TPT Verify should prove a reachable division by zero below. Guard it\n\
// like `apply_discount` does and re-run to see that finding clear, while\n\
// the precondition itself still gets checked separately.\n\
\n\
fn checkout_total(cents: i64, item_count: i64) -> i64 {\n\
    cents / item_count\n\
}\n\
\n\
fn apply_discount(cents: i64, divisor: i64) -> i64 {\n\
    assert!(divisor != 0);\n\
    cents / divisor\n\
}\n\
";

#[derive(Clone)]
enum Msg {
    SourceChanged(String),
    Analyze,
    #[cfg(feature = "pro")]
    DownloadReport,
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
    let source = Signal::new(DEFAULT_SOURCE.to_string());
    let outcome = Signal::new(Outcome::Idle);

    let view: Rc<dyn Fn() -> UITree<Msg>> = {
        let source = source.clone();
        let outcome = outcome.clone();
        Rc::new(move || build_view(&source, &outcome))
    };

    let dispatch: Rc<dyn Fn(Msg)> = {
        let source = source.clone();
        let outcome = outcome.clone();
        Rc::new(move |msg| match msg {
            Msg::SourceChanged(s) => source.set(s),
            Msg::Analyze => {
                let src = source.get();
                let next = match analyze_rust_source(&src) {
                    Ok(reports) => Outcome::Analyzed(Rc::new(RunReports { reports })),
                    Err(e) => Outcome::ParseError(e),
                };
                outcome.set(next);
            }
            #[cfg(feature = "pro")]
            Msg::DownloadReport => {
                if let Outcome::Analyzed(run) = outcome.get() {
                    let text = tpt_verify_engine::render_report("pasted source", &run.reports);
                    let _ = trigger_download(&text, "tpt-verify-report.txt");
                }
            }
        })
    };

    let handle = tpt_appfront_dom::render(container, view, dispatch)?;
    // Whole-process root mount: forgetting is an explicit choice here, not
    // the default — this handle must live as long as the tab does.
    std::mem::forget(handle);
    Ok(())
}

fn build_view(source: &Signal<String>, outcome: &Signal<Outcome>) -> UITree<Msg> {
    UITree::container(|c| {
        c.heading(1, "TPT Verify");
        c.text(
            "Paste one or more small Rust functions and check them for reachable \
             division-by-zero and violated assert!()s — entirely in your browser, nothing is \
             uploaded. Supported subset: integer parameters, 'let' bindings, assert!/\
             debug_assert!, and +-*/ arithmetic — see the note at the bottom.",
        );
        c.textarea(source.get()).on_input(Msg::SourceChanged);
        c.button("Analyze").on_click(Msg::Analyze);

        #[cfg(feature = "pro")]
        {
            if matches!(outcome.get(), Outcome::Analyzed(_)) {
                c.button("Download report").on_click(Msg::DownloadReport);
            }
        }

        match outcome.get() {
            Outcome::Idle => {
                c.text("No analysis run yet — click Analyze.");
            }
            Outcome::ParseError(err) => {
                c.text(format!("Couldn't parse this as Rust: {err}"));
            }
            Outcome::Analyzed(run) => render_reports(c, &run.reports),
        }

        c.text(
            "Note: division-by-zero freedom is proven exactly when it follows a matching \
             assert!/debug_assert! guard on the same value. Assertions themselves are checked \
             for reachable violation (a real, useful finding — it means a caller can trigger a \
             panic), but reasoning about compound arithmetic beyond a direct guard is \
             best-effort: a flagged assertion is worth a manual look, not an automatic confirmed \
             bug, and an unflagged compound assertion isn't yet a formal proof.",
        );
    })
}

fn render_reports(c: &mut ContainerBuilder<Msg>, reports: &[FunctionReport]) {
    c.text(format!("Analyzed {} function(s):", reports.len()));
    c.list(|l| {
        for r in reports {
            if let Some(err) = &r.error {
                l.text(format!("fn {}({}) — NOT ANALYZED: {}", r.name, r.params.join(", "), err));
                continue;
            }
            let Some(result) = &r.result else { continue };
            if let Some(parse_err) = &result.parse_error {
                l.text(format!("fn {}({}) — parse error: {}", r.name, r.params.join(", "), parse_err));
                continue;
            }
            if result.findings.is_empty() {
                l.text(format!(
                    "fn {}({}) — clean across {} feasible path(s).",
                    r.name,
                    r.params.join(", "),
                    result.paths_explored
                ));
            } else {
                l.text(format!(
                    "fn {}({}) — {} finding(s):",
                    r.name,
                    r.params.join(", "),
                    result.findings.len()
                ));
                for finding in &result.findings {
                    let tag = match finding.kind {
                        FindingKind::DivByZero => "[Division by zero]",
                        FindingKind::Assertion => "[Assertion]",
                        FindingKind::RangeOverflow => "[Range]",
                    };
                    l.text(format!("    {tag} {} — {}", finding.summary, finding.detail));
                }
            }
        }
    });
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
