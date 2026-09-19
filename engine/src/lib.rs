//! TPT Verify engine — a tiny verification DSL over `tpt-for-symbolic-exec`
//! (division-by-zero / broken-assertion detection) and, in the Pro edition,
//! `tpt-for-abstract-interp` (interval-based range/overflow checking).
//!
//! Pure Rust, no UI dependencies — host-testable (`cargo test`) and
//! wasm-compatible without changes.
//!
//! **Known engine limitation (from `tpt-for-symbolic-exec`'s ground SMT
//! encoding, not this crate):** conditions built from `+`/`-` and
//! comparisons are decided exactly. A condition whose arithmetic includes
//! `*` or `/` can't be turned into a ground SMT term at all, and the
//! underlying `feasible()` check falls back to treating it as reachable —
//! i.e. it never falsely claims a path is safe, but it also can't *prove*
//! safety for anything nonlinear yet. Multiplication/division still work
//! fine as ordinary arithmetic (and division is still checked for
//! zero-denominators); it's specifically *conditions containing them*
//! (`assert`/`assume`) that fall back conservatively. Worth surfacing to
//! users in the UI so a "no defects found" result on a program with
//! multiplication in its asserts isn't over-read as a proof.

mod parser;
#[cfg(feature = "pro")]
mod pdf;
#[cfg(feature = "pro")]
mod range;
mod report;
mod rust_frontend;

pub use parser::ParseError;
#[cfg(feature = "pro")]
pub use report::render_report_pdf;
pub use report::render_report;
pub use rust_frontend::{analyze_rust_source, FunctionReport};

use tpt_for_symbolic_exec::{run as sym_run, SCond, SExpr, SStmt, ViolationKind};

/// One human-readable finding surfaced to the UI.
#[derive(Clone, Debug)]
pub struct Finding {
    pub kind: FindingKind,
    pub summary: String,
    pub detail: String,
    /// A plain-English sentence describing the same witness path
    /// condition `detail` gives as raw boolean algebra (`x == 0 AND ...`)
    /// — for someone who doesn't want to parse solver notation to trust
    /// the finding. Both are kept: `detail` for someone who *does* want
    /// the precise expression.
    pub explanation: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingKind {
    DivByZero,
    Assertion,
    RangeOverflow,
}

/// The result of analyzing one program.
#[derive(Clone, Debug)]
pub struct AnalysisResult {
    /// Set when the source didn't parse; nothing else is populated.
    pub parse_error: Option<String>,
    pub paths_explored: usize,
    pub findings: Vec<Finding>,
}

impl AnalysisResult {
    pub fn is_clean(&self) -> bool {
        self.parse_error.is_none() && self.findings.is_empty()
    }
}

/// Runs symbolic execution (free + Pro) and, under `--features pro`, interval
/// range analysis too, over `source` written in TPT Verify's small
/// line-based DSL (see `parser` for the grammar).
pub fn analyze(source: &str) -> AnalysisResult {
    match parser::parse(source) {
        Ok(stmts) => analyze_stmts(&stmts),
        Err(e) => AnalysisResult {
            parse_error: Some(e.to_string()),
            paths_explored: 0,
            findings: Vec::new(),
        },
    }
}

/// The shared analysis core: runs symbolic execution (and, under `pro`,
/// interval range analysis) over an already-built statement list. Used by
/// both the line-DSL front end (`analyze`) and the Rust-source front end
/// (`rust_frontend`), so both surfaces get identical engine behavior.
pub(crate) fn analyze_stmts(stmts: &[SStmt]) -> AnalysisResult {
    let report = sym_run(stmts);
    #[cfg_attr(not(feature = "pro"), allow(unused_mut))]
    let mut findings: Vec<Finding> = report
        .violations
        .iter()
        .map(|v| {
            let (kind, summary) = match v.kind {
                ViolationKind::DivByZero => (FindingKind::DivByZero, "Division by zero is reachable".to_string()),
                ViolationKind::Assertion => (FindingKind::Assertion, "An assertion can be violated".to_string()),
            };
            Finding {
                kind,
                summary,
                detail: format!("witness path condition: {}", describe_path(&v.path_condition)),
                explanation: describe_path_natural(&v.path_condition),
            }
        })
        .collect();

    #[cfg(feature = "pro")]
    {
        findings.extend(range::range_findings(stmts));
    }

    AnalysisResult {
        parse_error: None,
        paths_explored: report.paths_explored,
        findings,
    }
}

fn describe_path(path: &[SCond]) -> String {
    if path.is_empty() {
        return "(always reachable)".to_string();
    }
    path.iter().map(describe_cond).collect::<Vec<_>>().join(" AND ")
}

fn describe_cond(c: &SCond) -> String {
    match c {
        SCond::True => "true".to_string(),
        SCond::Eq(a, b) => format!("{} == {}", describe_expr(a), describe_expr(b)),
        SCond::Lt(a, b) => format!("{} < {}", describe_expr(a), describe_expr(b)),
        SCond::Le(a, b) => format!("{} <= {}", describe_expr(a), describe_expr(b)),
        SCond::Gt(a, b) => format!("{} > {}", describe_expr(a), describe_expr(b)),
        SCond::Ge(a, b) => format!("{} >= {}", describe_expr(a), describe_expr(b)),
        SCond::Not(inner) => format!("NOT ({})", describe_cond(inner)),
        SCond::And(a, b) => format!("({}) AND ({})", describe_cond(a), describe_cond(b)),
        SCond::Or(a, b) => format!("({}) OR ({})", describe_cond(a), describe_cond(b)),
    }
}

/// A plain-English rendering of a witness path condition — see
/// `Finding::explanation`'s doc for why this exists alongside the raw
/// boolean form.
fn describe_path_natural(path: &[SCond]) -> String {
    if path.is_empty() {
        return "This is always reachable — no conditions restrict it.".to_string();
    }
    format!(
        "This is reachable when {}.",
        path.iter().map(describe_cond_natural).collect::<Vec<_>>().join(", and ")
    )
}

fn describe_cond_natural(c: &SCond) -> String {
    match c {
        SCond::True => "true".to_string(),
        SCond::Eq(a, b) => format!("{} equals {}", describe_expr(a), describe_expr(b)),
        SCond::Lt(a, b) => format!("{} is less than {}", describe_expr(a), describe_expr(b)),
        SCond::Le(a, b) => format!("{} is at most {}", describe_expr(a), describe_expr(b)),
        SCond::Gt(a, b) => format!("{} is greater than {}", describe_expr(a), describe_expr(b)),
        SCond::Ge(a, b) => format!("{} is at least {}", describe_expr(a), describe_expr(b)),
        // `!=` lowers to `Not(Eq(...))` — special-cased since it's by far
        // the most common negation this tool ever actually sees (division
        // guards), and "x is not equal to 0" reads far better than "it is
        // not the case that x equals 0". A *violated* `!=` guard doubles
        // that to `Not(Not(Eq(...)))` (the witness path negates the
        // guard condition itself) — eliminated back down to a plain
        // "x equals 0" rather than the double negative "it is not the
        // case that x is not equal to 0" that would otherwise fall out of
        // naively recursing.
        SCond::Not(inner) => match &**inner {
            SCond::Eq(a, b) => format!("{} is not equal to {}", describe_expr(a), describe_expr(b)),
            SCond::Not(innermost) => describe_cond_natural(innermost),
            // De Morgan for the four ordering comparisons too, so e.g. a
            // violated `assert!(x > 5)` reads as "x is at most 5" instead
            // of the grammatically-fine-but-stilted "it is not the case
            // that x is greater than 5".
            SCond::Lt(a, b) => format!("{} is at least {}", describe_expr(a), describe_expr(b)),
            SCond::Le(a, b) => format!("{} is greater than {}", describe_expr(a), describe_expr(b)),
            SCond::Gt(a, b) => format!("{} is at most {}", describe_expr(a), describe_expr(b)),
            SCond::Ge(a, b) => format!("{} is less than {}", describe_expr(a), describe_expr(b)),
            other => format!("it is not the case that {}", describe_cond_natural(other)),
        },
        SCond::And(a, b) => format!("{} and {}", describe_cond_natural(a), describe_cond_natural(b)),
        SCond::Or(a, b) => format!("either {} or {}", describe_cond_natural(a), describe_cond_natural(b)),
    }
}

fn describe_expr(e: &SExpr) -> String {
    match e {
        SExpr::Const(n) => n.to_string(),
        SExpr::Sym(s) => s.clone(),
        SExpr::Var(v) => v.clone(),
        SExpr::Add(a, b) => format!("({} + {})", describe_expr(a), describe_expr(b)),
        SExpr::Sub(a, b) => format!("({} - {})", describe_expr(a), describe_expr(b)),
        SExpr::Mul(a, b) => format!("({} * {})", describe_expr(a), describe_expr(b)),
        SExpr::Div(a, b) => format!("({} / {})", describe_expr(a), describe_expr(b)),
        SExpr::Neg(a) => format!("(-{})", describe_expr(a)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_div_by_zero() {
        let result = analyze("input x\nz = 10 / x\n");
        assert!(result.parse_error.is_none());
        assert!(result.findings.iter().any(|f| f.kind == FindingKind::DivByZero));
    }

    #[test]
    fn clean_when_guarded() {
        let result = analyze("input x\nassume x != 0\nz = 10 / x\n");
        assert!(result.is_clean(), "expected clean, got {:?}", result.findings);
    }

    #[test]
    fn detects_broken_assertion() {
        let result = analyze("input x\nassert x > 0\n");
        assert!(result.parse_error.is_none());
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].kind, FindingKind::Assertion);
    }

    #[test]
    fn reports_parse_error_for_undeclared_var() {
        let result = analyze("z = y + 1\n");
        assert!(result.parse_error.is_some());
    }

    #[test]
    fn direct_equality_guard_is_decided_exactly() {
        // The underlying solver reliably proves/disproves a direct
        // contradiction on the same symbolic value (the exact shape a
        // division-by-zero guard needs: `assume x != 0` vs. checking
        // `x == 0`) — this is the load-bearing case for the whole tool and
        // is exact, not conservative.
        let result = analyze("input x\nassume x == 2\nassert x == 2\n");
        assert!(result.is_clean(), "expected clean, got {:?}", result.findings);
    }

    #[test]
    fn compound_reasoning_is_conservative_not_unsound() {
        // Anything beyond a direct same-term contradiction — inequality
        // chaining (`x > 5` implying `x > 0`) or reasoning about a derived
        // expression (`x + 1 > x`) — isn't decided by the underlying
        // "minimal evaluator" SMT backend; it reports these as possibly
        // violated rather than silently assuming they're safe. That's the
        // right direction to be wrong in for a verification tool (it never
        // claims a false guarantee), but it does mean assertion-checking on
        // compound conditions is best-effort, not a proof, until a stronger
        // backend is wired in — surfaced to the user in the UI, not hidden.
        let result = analyze("input x\nassume x > 5\nassert x > 0\n");
        assert!(result.parse_error.is_none());
        assert!(!result.findings.is_empty());
    }

    #[test]
    fn explanation_reads_as_plain_english() {
        let result = analyze("input x\nz = 10 / x\n");
        let finding = &result.findings[0];
        assert!(
            finding.explanation.contains("is not equal to") || finding.explanation.contains("equals"),
            "expected plain-English wording, got: {}",
            finding.explanation
        );
        assert!(!finding.explanation.contains("AND"), "should read as prose, not boolean algebra: {}", finding.explanation);
    }

    #[test]
    fn violated_ne_guard_explanation_has_no_double_negative() {
        // `assert x != 0` violated -> witness is `Not(Not(Eq(x, 0)))`.
        // The explanation should read "x equals 0", not "it is not the
        // case that x is not equal to 0".
        let result = analyze("input x\nassert x != 0\n");
        let finding = &result.findings[0];
        assert_eq!(finding.explanation, "This is reachable when x equals 0.");
    }

    #[test]
    fn violated_gt_guard_reads_as_a_plain_comparison() {
        // `assert x > 5` violated -> witness is `Not(Gt(x, 5))`, which
        // should read as "x is at most 5", not a double-negative.
        let result = analyze("input x\nassert x > 5\n");
        let finding = &result.findings[0];
        assert_eq!(finding.explanation, "This is reachable when x is at most 5.");
    }
}
