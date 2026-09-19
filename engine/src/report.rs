//! Plain-text verification report export (Pro edition).
//!
//! Deliberately plain text/Markdown rather than a binary PDF: it's honest
//! about being a generated artifact, opens in literally anything, and is
//! easy to diff/version-control alongside the code it was run against —
//! all reasonable properties for something handed to an auditor or a
//! teammate as evidence. A branded PDF layout is a fast-follow, not a
//! blocker (`todo.md`).

use crate::FunctionReport;
#[cfg(feature = "pro")]
use crate::pdf::{render_pdf, Row};

/// Renders a full report across every function in a Rust-source analysis
/// run, suitable for saving as a `.md`/`.txt` file.
pub fn render_report(source_label: &str, reports: &[FunctionReport]) -> String {
    let mut out = String::new();
    out.push_str("TPT Verify — Verification Report\n");
    out.push_str("=================================\n\n");
    out.push_str(&format!("Source: {source_label}\n"));
    out.push_str(&format!("Functions analyzed: {}\n\n", reports.len()));

    let clean = reports.iter().filter(|r| r.error.is_none() && r.result.as_ref().is_some_and(|res| res.is_clean())).count();
    let with_findings = reports
        .iter()
        .filter(|r| r.result.as_ref().is_some_and(|res| !res.is_clean()))
        .count();
    let unsupported = reports.iter().filter(|r| r.error.is_some()).count();

    out.push_str(&format!(
        "Summary: {clean} clean, {with_findings} with findings, {unsupported} not analyzable (unsupported construct)\n\n"
    ));
    out.push_str("---\n\n");

    for r in reports {
        out.push_str(&format!("## fn {}({})\n\n", r.name, r.params.join(", ")));
        match (&r.error, &r.result) {
            (Some(err), _) => {
                out.push_str(&format!("STATUS: NOT ANALYZED — {err}\n\n"));
            }
            (None, Some(result)) => {
                if let Some(parse_err) = &result.parse_error {
                    out.push_str(&format!("STATUS: PARSE ERROR — {parse_err}\n\n"));
                    continue;
                }
                if result.findings.is_empty() {
                    out.push_str(&format!(
                        "STATUS: CLEAN — no defects found across {} feasible path(s).\n\n",
                        result.paths_explored
                    ));
                } else {
                    out.push_str(&format!(
                        "STATUS: {} FINDING(S) across {} feasible path(s):\n\n",
                        result.findings.len(),
                        result.paths_explored
                    ));
                    for f in &result.findings {
                        out.push_str(&format!("  - [{:?}] {}\n    {}\n    {}\n", f.kind, f.summary, f.detail, f.explanation));
                    }
                    out.push('\n');
                }
            }
            (None, None) => {
                out.push_str("STATUS: NOT ANALYZED\n\n");
            }
        }
    }

    out.push_str("---\n\n");
    out.push_str(
        "Methodology: division-by-zero freedom is proven by symbolic execution with an SMT \
         backend (exact for direct guards on the divisor). Assertion checks on compound \
         arithmetic are best-effort — a flagged assertion means \"reachable under this \
         engine's current reasoning\", not a confirmed defect; an unflagged assertion on \
         non-trivial arithmetic is not yet a proof. See the TPT Verify README for the exact \
         decidable fragment.\n",
    );

    out
}

/// Renders a full report across every function in a Rust-source analysis
/// run as a PDF (bytes), suitable for saving as a `.pdf` file. Same content
/// as [`render_report`], laid out with `crate::pdf`'s dependency-free
/// writer instead of plain text.
#[cfg(feature = "pro")]
pub fn render_report_pdf(source_label: &str, reports: &[FunctionReport]) -> Vec<u8> {
    let mut rows = vec![
        Row::Title("TPT Verify — Verification Report".to_string()),
        Row::Note(format!("Source: {source_label}")),
        Row::Gap(4.0),
    ];

    let clean = reports.iter().filter(|r| r.error.is_none() && r.result.as_ref().is_some_and(|res| res.is_clean())).count();
    let with_findings = reports
        .iter()
        .filter(|r| r.result.as_ref().is_some_and(|res| !res.is_clean()))
        .count();
    let unsupported = reports.iter().filter(|r| r.error.is_some()).count();
    rows.push(Row::Body(format!(
        "{} function(s) analyzed — {clean} clean, {with_findings} with findings, {unsupported} not analyzable",
        reports.len()
    )));
    rows.push(Row::Gap(10.0));

    for r in reports {
        rows.push(Row::Head(format!("fn {}({})", r.name, r.params.join(", "))));
        match (&r.error, &r.result) {
            (Some(err), _) => rows.push(Row::Body(format!("NOT ANALYZED — {err}"))),
            (None, Some(result)) => {
                if let Some(parse_err) = &result.parse_error {
                    rows.push(Row::Body(format!("PARSE ERROR — {parse_err}")));
                } else if result.findings.is_empty() {
                    rows.push(Row::Body(format!("CLEAN — no defects found across {} feasible path(s).", result.paths_explored)));
                } else {
                    rows.push(Row::Body(format!("{} finding(s) across {} feasible path(s):", result.findings.len(), result.paths_explored)));
                    rows.push(Row::Table {
                        columns: vec!["Kind".to_string(), "Summary".to_string(), "Witness / explanation".to_string()],
                        rows: result
                            .findings
                            .iter()
                            .map(|f| vec![format!("{:?}", f.kind), f.summary.clone(), format!("{} | {}", f.detail, f.explanation)])
                            .collect(),
                        weights: Some(vec![0.16, 0.3, 0.54]),
                    });
                }
            }
            (None, None) => rows.push(Row::Body("NOT ANALYZED".to_string())),
        }
        rows.push(Row::Gap(8.0));
    }

    rows.push(Row::Gap(4.0));
    rows.push(Row::Note(
        "Methodology: division-by-zero freedom is proven by symbolic execution with an SMT \
         backend (exact for direct guards on the divisor). Assertion checks on compound \
         arithmetic are best-effort. Range findings (Pro) use real interval fixpoint analysis."
            .to_string(),
    ));

    render_pdf("TPT Verify — Verification Report", &rows)
}
