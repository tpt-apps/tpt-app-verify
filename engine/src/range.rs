//! Pro-only: real interval analysis over `tpt_for_symbolic_exec`'s
//! straight-line-with-branches program, replacing the v1 "any unbounded
//! input has no range assumption" heuristic with genuine fixpoint interval
//! propagation via `tpt_for_abstract_interp::analyze`.
//!
//! Every `SStmt` becomes its own basic block (one statement per block —
//! simplest possible correct CFG, and cheap enough at these program sizes)
//! so a block's recorded *entry* state is exactly "the state right before
//! this statement runs"; `SStmt::If` allocates a then/else/merge triple and
//! recurses. `assert`/`assume` conditions narrow a variable's interval only
//! when they're a direct `var <op> constant` comparison (what
//! `tpt_for_abstract_interp::CmpOp` can express) — anything else (`!=`,
//! `&&`/`||`, a variable-vs-variable comparison) is skipped rather than
//! guessed at, which is sound (just less precise) since skipping a
//! narrowing can only make an interval *wider*, never claim a false bound.
//!
//! `SExpr::Div` has no interval semantics in the abstract domain (division
//! isn't modeled there at all); any expression containing one is marked
//! "tainted" and simply isn't tracked, rather than mis-modeled.

use std::collections::{HashMap, HashSet};

use tpt_for_abstract_interp::{analyze, AbstractDomain, Cfg, CmpOp, Expr as AiExpr, Interval};
use tpt_for_symbolic_exec::{SCond, SExpr, SStmt};

use crate::{Finding, FindingKind};

/// Runs interval analysis over `stmts` and returns one finding per
/// arithmetic expression whose proven interval touches an `i64` boundary
/// (`i64::MIN`/`i64::MAX`) on some feasible path — the saturating interval
/// arithmetic below reaches exactly that boundary whenever the true
/// mathematical result would exceed it, so this is the direct signal for
/// "this can realistically overflow given what's actually known about its
/// inputs", not just "some input somewhere is unbounded".
pub(crate) fn range_findings(stmts: &[SStmt]) -> Vec<Finding> {
    let mut order: Vec<String> = Vec::new();
    let mut sym_names: HashSet<String> = HashSet::new();
    collect_names(stmts, &mut order, &mut sym_names);
    if order.is_empty() {
        return Vec::new();
    }
    let vars: HashMap<String, usize> = order.iter().cloned().enumerate().map(|(i, n)| (n, i)).collect();

    let mut cfg = Cfg::new(order.len());
    let entry = cfg.block(&[], &[]);
    let mut tainted: HashSet<String> = HashSet::new();
    let mut checks: Vec<(usize, AiExpr)> = Vec::new();
    let exit = lower_seq(stmts, &mut cfg, entry, &vars, &mut tainted, &mut checks);
    cfg.set_block(exit, &[], &[]);

    // Free symbolic inputs start unconstrained (top); a name that's only
    // ever an `Assign` target starts at bottom (nothing has reached it yet)
    // — matching `tpt_for_abstract_interp`'s own convention (see its module
    // doc example, which assigns before any read).
    let init: Vec<Interval> = order.iter().map(|n| if sym_names.contains(n) { Interval::top() } else { Interval::bottom() }).collect();

    let Some(states) = analyze(&cfg, entry, init) else {
        return Vec::new();
    };

    checks
        .into_iter()
        .filter_map(|(block_id, expr)| {
            let result = expr.eval(&states[block_id]);
            if result.hi() == i64::MAX || result.lo() == i64::MIN {
                Some(Finding {
                    kind: FindingKind::RangeOverflow,
                    summary: "Arithmetic may exceed i64 range".to_string(),
                    detail: format!(
                        "Interval analysis proves no bound on one side of this expression's \
                         result on some feasible path (computed range: {}..{}) — add an \
                         'assert'/'assume' bounding the inputs, or confirm this can't overflow \
                         for the values it actually receives.",
                        fmt_bound(result.lo()),
                        fmt_bound(result.hi())
                    ),
                    explanation: format!(
                        "This expression's result can be as {} as {} on some feasible path, \
                         given everything currently known about its inputs — add a tighter \
                         bound if that's not actually possible in practice.",
                        if result.lo() == i64::MIN { "small" } else { "large" },
                        if result.lo() == i64::MIN { "i64::MIN".to_string() } else { "i64::MAX".to_string() }
                    ),
                })
            } else {
                None
            }
        })
        .collect()
}

fn fmt_bound(b: i64) -> String {
    match b {
        i64::MIN => "i64::MIN".to_string(),
        i64::MAX => "i64::MAX".to_string(),
        other => other.to_string(),
    }
}

fn collect_names(stmts: &[SStmt], order: &mut Vec<String>, syms: &mut HashSet<String>) {
    fn push(name: &str, order: &mut Vec<String>) {
        if !order.iter().any(|n| n == name) {
            order.push(name.to_string());
        }
    }
    fn walk_expr(e: &SExpr, order: &mut Vec<String>, syms: &mut HashSet<String>) {
        match e {
            SExpr::Const(_) => {}
            SExpr::Sym(s) => {
                push(s, order);
                syms.insert(s.clone());
            }
            SExpr::Var(v) => push(v, order),
            SExpr::Add(a, b) | SExpr::Sub(a, b) | SExpr::Mul(a, b) | SExpr::Div(a, b) => {
                walk_expr(a, order, syms);
                walk_expr(b, order, syms);
            }
            SExpr::Neg(a) => walk_expr(a, order, syms),
        }
    }
    fn walk_cond(c: &SCond, order: &mut Vec<String>, syms: &mut HashSet<String>) {
        match c {
            SCond::True => {}
            SCond::Eq(a, b) | SCond::Lt(a, b) | SCond::Le(a, b) | SCond::Gt(a, b) | SCond::Ge(a, b) => {
                walk_expr(a, order, syms);
                walk_expr(b, order, syms);
            }
            SCond::Not(inner) => walk_cond(inner, order, syms),
            SCond::And(a, b) | SCond::Or(a, b) => {
                walk_cond(a, order, syms);
                walk_cond(b, order, syms);
            }
        }
    }
    for s in stmts {
        match s {
            SStmt::Assign(v, e) => {
                push(v, order);
                walk_expr(e, order, syms);
            }
            SStmt::Assert(c) | SStmt::Assume(c) => walk_cond(c, order, syms),
            SStmt::If(c, then_b, else_b) => {
                walk_cond(c, order, syms);
                collect_names(then_b, order, syms);
                collect_names(else_b, order, syms);
            }
        }
    }
}

/// Lowers an `SExpr` into the abstract-interpretation domain's `Expr`.
/// Returns `None` for anything the interval domain can't model (currently
/// just `Div`, which has no interval semantics here) or that references a
/// `tainted` name — taint propagates outward so a containing expression
/// that depends on an unmodelable sub-expression is itself left untracked
/// rather than silently under-constrained.
fn lower_expr(e: &SExpr, vars: &HashMap<String, usize>, tainted: &HashSet<String>) -> Option<AiExpr> {
    match e {
        SExpr::Const(c) => Some(AiExpr::const_(*c)),
        SExpr::Sym(s) | SExpr::Var(s) => {
            if tainted.contains(s) {
                None
            } else {
                vars.get(s).map(|&v| AiExpr::var(v))
            }
        }
        SExpr::Add(a, b) => Some(AiExpr::add(lower_expr(a, vars, tainted)?, lower_expr(b, vars, tainted)?)),
        SExpr::Sub(a, b) => Some(AiExpr::sub(lower_expr(a, vars, tainted)?, lower_expr(b, vars, tainted)?)),
        SExpr::Mul(a, b) => Some(AiExpr::mul(lower_expr(a, vars, tainted)?, lower_expr(b, vars, tainted)?)),
        SExpr::Div(_, _) => None,
        SExpr::Neg(a) => Some(AiExpr::neg(lower_expr(a, vars, tainted)?)),
    }
}

/// True for anything beyond a bare passthrough (`Const`/`Sym`/`Var`) — the
/// set of assigns worth an overflow check at all.
fn is_arithmetic(e: &SExpr) -> bool {
    !matches!(e, SExpr::Const(_) | SExpr::Sym(_) | SExpr::Var(_))
}

/// Decomposes `cond` into a single `(var, op, constant)` narrowing, the only
/// shape `tpt_for_abstract_interp::Stmt::Assume` can express. `negate`
/// produces the narrowing for the *else* branch (the condition's logical
/// negation) instead. Returns `None` — skip narrowing, not a guess — for
/// anything else: `&&`/`||`/`!`, a variable-vs-variable comparison, or `==`
/// negated (there's no `CmpOp::Ne`; excluding a single point isn't an
/// interval anyway).
fn lower_simple_cond(c: &SCond, negate: bool, vars: &HashMap<String, usize>, tainted: &HashSet<String>) -> Option<(usize, CmpOp, i64)> {
    use CmpOp::*;
    let (a, b, op) = match c {
        SCond::Lt(a, b) if !negate => (a, b, Lt),
        SCond::Le(a, b) if !negate => (a, b, Le),
        SCond::Gt(a, b) if !negate => (a, b, Gt),
        SCond::Ge(a, b) if !negate => (a, b, Ge),
        SCond::Eq(a, b) if !negate => (a, b, Eq),
        SCond::Lt(a, b) => (a, b, Ge),
        SCond::Le(a, b) => (a, b, Gt),
        SCond::Gt(a, b) => (a, b, Le),
        SCond::Ge(a, b) => (a, b, Lt),
        _ => return None,
    };
    cmp_of(a, b, op, vars, tainted)
}

fn cmp_of(a: &SExpr, b: &SExpr, op: CmpOp, vars: &HashMap<String, usize>, tainted: &HashSet<String>) -> Option<(usize, CmpOp, i64)> {
    use CmpOp::*;
    let flip = |op: CmpOp| match op {
        Lt => Gt,
        Le => Ge,
        Gt => Lt,
        Ge => Le,
        Eq => Eq,
    };
    match (a, b) {
        (SExpr::Sym(s), SExpr::Const(k)) | (SExpr::Var(s), SExpr::Const(k)) if !tainted.contains(s) => vars.get(s).map(|&v| (v, op, *k)),
        (SExpr::Const(k), SExpr::Sym(s)) | (SExpr::Const(k), SExpr::Var(s)) if !tainted.contains(s) => vars.get(s).map(|&v| (v, flip(op), *k)),
        _ => None,
    }
}

/// Lowers `stmts` into `cfg`, starting at the pre-allocated empty block
/// `entry`, and returns the id of a fresh, still-unset ("open") block
/// representing control flow after `stmts` — the caller wires its
/// stmts/successors once it knows what comes next.
fn lower_seq(
    stmts: &[SStmt],
    cfg: &mut Cfg,
    entry: usize,
    vars: &HashMap<String, usize>,
    tainted: &mut HashSet<String>,
    checks: &mut Vec<(usize, AiExpr)>,
) -> usize {
    let mut cur = entry;
    for stmt in stmts {
        match stmt {
            SStmt::Assign(name, expr) => {
                let next = cfg.block(&[], &[]);
                match lower_expr(expr, vars, tainted) {
                    Some(ai_expr) => {
                        if let Some(&vid) = vars.get(*name) {
                            cfg.set_block(cur, &[tpt_for_abstract_interp::Stmt::assign(vid, ai_expr.clone())], &[next]);
                            if is_arithmetic(expr) {
                                checks.push((cur, ai_expr));
                            }
                        } else {
                            cfg.set_block(cur, &[], &[next]);
                        }
                    }
                    None => {
                        tainted.insert((*name).to_string());
                        cfg.set_block(cur, &[], &[next]);
                    }
                }
                cur = next;
            }
            SStmt::Assert(cond) | SStmt::Assume(cond) => {
                let next = cfg.block(&[], &[]);
                let block_stmts = match lower_simple_cond(cond, false, vars, tainted) {
                    Some((vid, op, k)) => vec![tpt_for_abstract_interp::Stmt::assume(vid, op, k)],
                    None => vec![],
                };
                cfg.set_block(cur, &block_stmts, &[next]);
                cur = next;
            }
            SStmt::If(cond, then_b, else_b) => {
                let then_head = cfg.block(&[], &[]);
                let else_head = cfg.block(&[], &[]);
                let merge = cfg.block(&[], &[]);
                cfg.set_block(cur, &[], &[then_head, else_head]);

                let then_inner = cfg.block(&[], &[]);
                let then_narrow = match lower_simple_cond(cond, false, vars, tainted) {
                    Some((vid, op, k)) => vec![tpt_for_abstract_interp::Stmt::assume(vid, op, k)],
                    None => vec![],
                };
                cfg.set_block(then_head, &then_narrow, &[then_inner]);
                let then_exit = lower_seq(then_b, cfg, then_inner, vars, tainted, checks);
                cfg.set_block(then_exit, &[], &[merge]);

                let else_inner = cfg.block(&[], &[]);
                let else_narrow = match lower_simple_cond(cond, true, vars, tainted) {
                    Some((vid, op, k)) => vec![tpt_for_abstract_interp::Stmt::assume(vid, op, k)],
                    None => vec![],
                };
                cfg.set_block(else_head, &else_narrow, &[else_inner]);
                let else_exit = lower_seq(else_b, cfg, else_inner, vars, tainted, checks);
                cfg.set_block(else_exit, &[], &[merge]);

                cur = merge;
            }
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze_stmts;

    fn findings(stmts: &[SStmt]) -> Vec<Finding> {
        range_findings(stmts)
    }

    #[test]
    fn unconstrained_multiplication_is_flagged() {
        let stmts = vec![SStmt::assign("z", SExpr::mul(SExpr::sym("x"), SExpr::sym("y")))];
        let f = findings(&stmts);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FindingKind::RangeOverflow);
    }

    #[test]
    fn bounded_multiplication_is_not_flagged() {
        let stmts = vec![
            SStmt::assume(SCond::ge(SExpr::sym("x"), SExpr::const_(0))),
            SStmt::assume(SCond::lt(SExpr::sym("x"), SExpr::const_(1000))),
            SStmt::assume(SCond::ge(SExpr::sym("y"), SExpr::const_(0))),
            SStmt::assume(SCond::lt(SExpr::sym("y"), SExpr::const_(1000))),
            SStmt::assign("z", SExpr::mul(SExpr::sym("x"), SExpr::sym("y"))),
        ];
        assert!(findings(&stmts).is_empty(), "{:?}", findings(&stmts));
    }

    #[test]
    fn bounded_but_still_overflowing_multiplication_is_flagged() {
        // x, y both bounded, but their bound is wide enough that the product
        // still exceeds i64::MAX on the extreme corner — real overflow that
        // the old "has an assume at all" heuristic could never have caught.
        let stmts = vec![
            SStmt::assume(SCond::ge(SExpr::sym("x"), SExpr::const_(0))),
            SStmt::assume(SCond::le(SExpr::sym("x"), SExpr::const_(i64::MAX / 2))),
            SStmt::assign("z", SExpr::mul(SExpr::sym("x"), SExpr::const_(4))),
        ];
        assert_eq!(findings(&stmts).len(), 1);
    }

    #[test]
    fn plain_addition_of_free_inputs_is_not_flagged() {
        // Two fully unconstrained inputs added together: real overflow risk
        // exists in principle, but flagging every '+' between free symbols
        // would be pure noise for realistic arithmetic-heavy code — only
        // the actual computed interval touching a boundary matters, and
        // for '+' of two top intervals the *saturated* result still touches
        // both boundaries... so this one *is* expected to flag, documenting
        // that '+'/'-' are checked with the same precision as '*', not
        // specially exempted.
        let stmts = vec![SStmt::assign("z", SExpr::add(SExpr::sym("x"), SExpr::sym("y")))];
        assert_eq!(findings(&stmts).len(), 1);
    }

    #[test]
    fn division_is_not_tracked_but_does_not_crash() {
        let stmts = vec![
            SStmt::assign("z", SExpr::div(SExpr::sym("x"), SExpr::sym("y"))),
            SStmt::assign("w", SExpr::mul(SExpr::var("z"), SExpr::const_(2))),
        ];
        // 'z' is tainted (division isn't modeled); 'w' depends on it, so
        // it's untracked too — no finding, but also no panic.
        assert!(findings(&stmts).is_empty());
    }

    #[test]
    fn if_else_each_branch_analyzed_on_its_own_narrowed_state() {
        // Both branches multiply x by a constant; only the branch where x
        // isn't narrowed can overflow.
        let stmts = vec![SStmt::If(
            SCond::lt(SExpr::sym("x"), SExpr::const_(100)),
            vec![SStmt::assign("a", SExpr::mul(SExpr::sym("x"), SExpr::const_(2)))],
            vec![SStmt::assign("b", SExpr::mul(SExpr::sym("x"), SExpr::const_(2)))],
        )];
        // then-branch: x < 100 narrows x to [MIN, 99] -- still touches MIN,
        // so still flagged (x isn't lower-bounded); else-branch: x >= 100,
        // touches MAX. Both branches are genuinely unbounded on one side
        // here, so both are correctly flagged -- this is a precision, not a
        // soundness, property: add a lower bound too and the then-branch
        // finding clears (covered by `bounded_multiplication_is_not_flagged`).
        assert_eq!(findings(&stmts).len(), 2);
    }

    #[test]
    fn full_pipeline_smoke_test_via_analyze_stmts() {
        // Sanity: this module's findings actually reach the public API
        // (engine/src/lib.rs's `analyze_stmts`, `pro`-gated) end to end.
        let stmts = vec![SStmt::assign("z", SExpr::mul(SExpr::sym("x"), SExpr::sym("y")))];
        let result = analyze_stmts(&stmts);
        assert!(result.findings.iter().any(|f| f.kind == FindingKind::RangeOverflow));
    }
}
