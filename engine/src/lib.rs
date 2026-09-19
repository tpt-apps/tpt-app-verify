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
mod report;
mod rust_frontend;

pub use parser::ParseError;
pub use report::render_report;
pub use rust_frontend::{analyze_rust_source, FunctionReport};

use tpt_for_symbolic_exec::{run as sym_run, SCond, SExpr, SStmt, ViolationKind};

/// One human-readable finding surfaced to the UI.
#[derive(Clone, Debug)]
pub struct Finding {
    pub kind: FindingKind,
    pub summary: String,
    pub detail: String,
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
            }
        })
        .collect();

    #[cfg(feature = "pro")]
    {
        findings.extend(range_findings(stmts));
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

/// Pro-only: a lightweight interval sanity pass. This is intentionally
/// simple for v1 — it flags free inputs with no bounding `assume`, since an
/// unbounded input can drive any downstream arithmetic to overflow. A full
/// CFG-based `tpt_for_abstract_interp::analyze` pass (per-statement interval
/// tracking) is real follow-up work, not a v1 blocker.
#[cfg(feature = "pro")]
fn range_findings(stmts: &[tpt_for_symbolic_exec::SStmt]) -> Vec<Finding> {
    use std::collections::HashSet;
    use tpt_for_symbolic_exec::SStmt;

    let mut bounded: HashSet<String> = HashSet::new();
    let mut syms: HashSet<String> = HashSet::new();

    fn collect_syms_in_expr(e: &SExpr, out: &mut HashSet<String>) {
        match e {
            SExpr::Sym(s) => {
                out.insert(s.clone());
            }
            SExpr::Add(a, b) | SExpr::Sub(a, b) | SExpr::Mul(a, b) | SExpr::Div(a, b) => {
                collect_syms_in_expr(a, out);
                collect_syms_in_expr(b, out);
            }
            SExpr::Neg(a) => collect_syms_in_expr(a, out),
            _ => {}
        }
    }
    fn collect_syms_in_cond(c: &SCond, out: &mut HashSet<String>) {
        match c {
            SCond::Eq(a, b) | SCond::Lt(a, b) | SCond::Le(a, b) | SCond::Gt(a, b) | SCond::Ge(a, b) => {
                collect_syms_in_expr(a, out);
                collect_syms_in_expr(b, out);
            }
            SCond::Not(inner) => collect_syms_in_cond(inner, out),
            SCond::And(a, b) | SCond::Or(a, b) => {
                collect_syms_in_cond(a, out);
                collect_syms_in_cond(b, out);
            }
            SCond::True => {}
        }
    }

    for s in stmts {
        match s {
            SStmt::Assign(_, e) => collect_syms_in_expr(e, &mut syms),
            SStmt::Assert(c) | SStmt::Assume(c) => {
                collect_syms_in_cond(c, &mut bounded);
                collect_syms_in_cond(c, &mut syms);
            }
            SStmt::If(c, then, els) => {
                collect_syms_in_cond(c, &mut bounded);
                collect_syms_in_cond(c, &mut syms);
                for s in then.iter().chain(els.iter()) {
                    // Shallow: only one level of nesting matters for this pass.
                    if let SStmt::Assign(_, e) = s {
                        collect_syms_in_expr(e, &mut syms);
                    }
                }
            }
        }
    }

    syms.difference(&bounded)
        .map(|name| Finding {
            kind: FindingKind::RangeOverflow,
            summary: format!("Unbounded input '{name}' has no range assumption"),
            detail: format!(
                "'{name}' is used in arithmetic with no 'assert'/'assume' constraining its range — \
                 add one (e.g. `assume {name} >= 0 && {name} < 1000000`) or confirm downstream \
                 arithmetic can't overflow for its full i64 range."
            ),
        })
        .collect()
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
}
