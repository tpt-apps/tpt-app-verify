//! A real Rust-source front end: parses actual `fn` items (via `syn`) and
//! lowers a supported subset of their bodies into `tpt_for_symbolic_exec`'s
//! `SStmt`/`SExpr`/`SCond` AST, then runs the same analysis core as the
//! line-DSL front end (`crate::analyze_stmts`).
//!
//! **Supported subset, checked exhaustively (never silently skipped):**
//! - Integer-typed parameters (`i32`/`i64`/`u32`/`u64`/`usize`/`isize`)
//!   become free symbolic inputs.
//! - `let name = expr;` bindings (no destructuring, no explicit type
//!   annotation requirement, no `else` diverge clause).
//! - `assert!(cond)` / `debug_assert!(cond)` (an optional format-string
//!   argument after the condition is accepted and ignored).
//! - Arithmetic: literals, identifiers, `+ - * /`, unary `-`, parens.
//! - Conditions: `== != < <= > >=`, `&& || !`, parens.
//!
//! - `if`/`else` (including `else if` chains): both branches are explored
//!   (the underlying `tpt_for_symbolic_exec::run` already does real
//!   branch-then-continue path exploration — this frontend only lowers
//!   `syn`'s `ExprIf` into its `SStmt::If`). An `if` used as a value (a
//!   `let` initializer or a function's tail expression) may feed further
//!   *arithmetic*, but not yet a later `assert!`/`if` condition — see
//!   `Ctx::var_backed` below for why that specific case is rejected
//!   explicitly rather than silently mis-analyzed.
//!
//! Anything outside this subset (loops, function calls, non-integer
//! types, struct/array/pointer access, ...) makes that *function* report an
//! error naming the unsupported construct — analysis never silently ignores
//! code it can't model, since a verification tool that does that is worse
//! than useless. Other functions in the same file are still analyzed.

use std::collections::{HashMap, HashSet};

use syn::parse::Parser;
use syn::{BinOp, Expr, FnArg, Item, Pat, Stmt, Type, UnOp};
use tpt_for_symbolic_exec::{SCond, SExpr, SStmt};

use crate::{analyze_stmts, AnalysisResult};

/// The synthetic variable a function's tail expression is checked through.
/// A string literal is already `'static` — no leak needed here, unlike the
/// user-named bindings `intern()` below has to handle.
const RETURN_VAR: &str = "__return";

/// The result of analyzing one `fn` item found in the source.
pub struct FunctionReport {
    pub name: String,
    pub params: Vec<String>,
    /// Set when this function's body uses a construct outside the
    /// supported subset (see the module doc). Nothing else is populated.
    pub error: Option<String>,
    pub result: Option<AnalysisResult>,
}

#[derive(Clone)]
struct Ctx {
    /// Function parameters — free symbolic inputs (`SExpr::Sym`).
    declared: HashSet<String>,
    /// Plain `let` bindings — resolved by inlining their (already-lowered)
    /// definition at every use site, not by reference. See the module doc
    /// on `parser`'s line-DSL front end for why: `Assert`/`Assume`/`If`
    /// conditions in the underlying engine are checked as-written, not
    /// evaluated through the interpreter's variable store the way an
    /// `Assign` right-hand side is, so a condition built from a bare
    /// `SExpr::Var` would be checked against an unconstrained value
    /// instead of its actual definition.
    defs: HashMap<String, SExpr>,
    /// `let` bindings whose value came from an `if`/`else` (so it can
    /// legitimately differ per branch) — these resolve to `SExpr::Var`
    /// instead of an inlined expression, since inlining would require a
    /// single conditional `SExpr` node that doesn't exist. `Var`s *do*
    /// resolve correctly through the store for further `Assign`
    /// right-hand sides (ordinary arithmetic), which is why this still
    /// works for e.g. `let z = if c { 1 } else { 2 }; let w = z + 1;` —
    /// but a condition can't see through the store, so referencing one of
    /// these names inside a later `assert!`/`if` is rejected explicitly
    /// (`expr_has_var`) rather than silently checking it against a free,
    /// disconnected symbol.
    var_backed: HashSet<String>,
}

/// Leaks each *distinct* variable name exactly once, process-wide, and
/// reuses the same `&'static str` for repeated occurrences — required
/// because `SStmt::Assign` takes `&'static str` (a `tpt_for_symbolic_exec`
/// API constraint, not this crate's choice) and analysis can run
/// arbitrarily many times in one long-lived process (every "Analyze" click
/// in the browser tab, with no page reload between them). Leaking fresh on
/// every call, as before, grew unboundedly with clicks; this caps growth to
/// the number of distinct identifier names ever seen, which for real
/// source is small and bounded.
fn intern(name: &str) -> &'static str {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<String, &'static str>> =
            std::cell::RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(&existing) = cache.get(name) {
            return existing;
        }
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        cache.insert(name.to_string(), leaked);
        leaked
    })
}

const SUPPORTED_INT_TYPES: &[&str] = &["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize"];

/// Parses `source` as a Rust file and analyzes every top-level `fn` item.
/// Returns one report per function; a whole-file parse error (invalid Rust
/// syntax) is returned as `Err` instead, since nothing can be analyzed then.
pub fn analyze_rust_source(source: &str) -> Result<Vec<FunctionReport>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("Rust parse error: {e}"))?;

    let fns: Vec<&syn::ItemFn> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(f) => Some(f),
            _ => None,
        })
        .collect();

    if fns.is_empty() {
        return Err("No `fn` items found — define at least one function.".to_string());
    }

    Ok(fns.into_iter().map(analyze_fn).collect())
}

fn analyze_fn(f: &syn::ItemFn) -> FunctionReport {
    let name = f.sig.ident.to_string();

    let mut ctx = Ctx {
        declared: HashSet::new(),
        defs: HashMap::new(),
        var_backed: HashSet::new(),
    };
    let mut params = Vec::new();

    for arg in &f.sig.inputs {
        match arg {
            FnArg::Typed(pat_type) => match (&*pat_type.pat, &*pat_type.ty) {
                (Pat::Ident(pat_ident), Type::Path(type_path)) => {
                    let ty_name = type_path.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                    if !SUPPORTED_INT_TYPES.contains(&ty_name.as_str()) {
                        return unsupported(name, params, format!(
                            "parameter '{}' has type '{}' — only integer types are supported ({})",
                            pat_ident.ident, ty_name, SUPPORTED_INT_TYPES.join(", ")
                        ));
                    }
                    let pname = pat_ident.ident.to_string();
                    ctx.declared.insert(pname.clone());
                    params.push(pname);
                }
                _ => {
                    return unsupported(name, params, "only simple typed identifier parameters are supported (no patterns, references, or generics)".to_string());
                }
            },
            FnArg::Receiver(_) => {
                return unsupported(name, params, "methods with a 'self' receiver aren't supported".to_string());
            }
        }
    }

    // The function's tail expression (its implicit return value, if any) is
    // checked through `RETURN_VAR` — see `lower_block`'s tail-position
    // handling. A return value isn't otherwise modeled (there's nothing to
    // assert about it beyond the arithmetic that produced it).
    let stmts: Vec<SStmt> = match lower_block(&f.block, &mut ctx, Some(RETURN_VAR)) {
        Ok(stmts) => stmts,
        Err(msg) => return unsupported(name, params, msg),
    };

    FunctionReport {
        name,
        params,
        error: None,
        result: Some(analyze_stmts(&stmts)),
    }
}

fn unsupported(name: String, params: Vec<String>, msg: String) -> FunctionReport {
    FunctionReport {
        name,
        params,
        error: Some(msg),
        result: None,
    }
}

/// Lowers a `{ ... }` block, correctly distinguishing its final
/// tail-position expression (which produces `target`'s value, if any) from
/// every earlier statement (lowered via `lower_stmt`, which never sees tail
/// position). Used for both a whole function body and each `if`/`else`
/// branch body — real Rust already scopes a branch's own `let` bindings to
/// that branch, which is why each branch gets its own cloned `Ctx` in
/// `lower_if_stmts` rather than sharing one across sibling branches.
fn lower_block(block: &syn::Block, ctx: &mut Ctx, target: Option<&'static str>) -> Result<Vec<SStmt>, String> {
    let mut out = Vec::new();
    let last_idx = block.stmts.len().checked_sub(1);
    for (i, stmt) in block.stmts.iter().enumerate() {
        if Some(i) == last_idx {
            if let Stmt::Expr(expr, None) = stmt {
                out.extend(lower_tail(expr, ctx, target)?);
                continue;
            }
        }
        out.extend(lower_stmt(stmt, ctx)?);
    }
    Ok(out)
}

/// Lowers a block's tail-position expression (no trailing `;`): either the
/// function's implicit return value, or an `if`/`else` branch's produced
/// value. `target` is `Some(var)` when this position is expected to
/// produce a value (checked through `var`, an `Assign` so division-by-zero
/// in it is still caught — never just discarded, which would silently skip
/// checking it) and `None` when this position is a `()`-typed statement
/// context (e.g. a top-level `if` with no value used).
fn lower_tail(expr: &Expr, ctx: &mut Ctx, target: Option<&'static str>) -> Result<Vec<SStmt>, String> {
    match (expr, target) {
        (Expr::Macro(m), _) => lower_assert_stmts(&m.mac, ctx),
        (Expr::If(if_expr), _) => lower_if_stmts(if_expr, ctx, target),
        (_, Some(t)) => {
            let value = lower_expr(expr, ctx)?;
            Ok(vec![SStmt::assign(t, value)])
        }
        (_, None) => Err(
            "this branch's trailing expression isn't supported (only 'if/else' or \
             'assert!'/'debug_assert!' can end a branch that isn't producing a value)"
                .to_string(),
        ),
    }
}

/// Lowers `if cond { then } else { els }` (or an `else if` chain, or no
/// `else` at all) into a single `SStmt::If`. The underlying
/// `tpt_for_symbolic_exec::run` already explores both branches for real
/// (cloning the store/path-condition down each side and pruning whichever
/// is infeasible) — this function's only job is producing that one AST
/// node from `syn`'s `ExprIf`.
///
/// `target`: see `lower_tail`. When `Some`, both branches must produce a
/// value (so `if` with no `else` is rejected — there'd be nothing to use
/// on the implicit "condition was false" path), and the produced value is
/// registered in `ctx.var_backed` by the caller (the value can differ per
/// branch, so it can't be inlined as one static expression).
fn lower_if_stmts(if_expr: &syn::ExprIf, ctx: &mut Ctx, target: Option<&'static str>) -> Result<Vec<SStmt>, String> {
    let cond = lower_cond(&if_expr.cond, ctx)?;

    let mut then_ctx = ctx.clone();
    let then_stmts = lower_block(&if_expr.then_branch, &mut then_ctx, target)?;

    let else_stmts = match &if_expr.else_branch {
        Some((_, else_expr)) => {
            let mut else_ctx = ctx.clone();
            match &**else_expr {
                Expr::If(inner) => lower_if_stmts(inner, &mut else_ctx, target)?,
                Expr::Block(b) => lower_block(&b.block, &mut else_ctx, target)?,
                _ => return Err("unsupported 'else' form (only 'else { ... }' or 'else if ...' are supported)".to_string()),
            }
        }
        None => {
            if target.is_some() {
                return Err("'if' without a matching 'else' can't be used as a value here".to_string());
            }
            Vec::new()
        }
    };

    Ok(vec![SStmt::if_then_else(cond, then_stmts, else_stmts)])
}

/// Lowers one non-tail-position statement to zero or more `SStmt`s. An
/// `assert!` lowers to *two* statements — `Assert` (report a finding if
/// it's reachably false) followed by `Assume` (narrow the path condition,
/// since real Rust semantics guarantee the condition holds for everything
/// after an assertion that didn't panic) — because `tpt-for-symbolic-exec`'s
/// `Assert` alone only checks a condition without remembering it, unlike an
/// actual `assert!` in a real program.
fn lower_stmt(stmt: &Stmt, ctx: &mut Ctx) -> Result<Vec<SStmt>, String> {
    match stmt {
        Stmt::Local(local) => {
            let name = match &local.pat {
                Pat::Ident(pi) => pi.ident.to_string(),
                Pat::Type(pt) => match &*pt.pat {
                    Pat::Ident(pi) => pi.ident.to_string(),
                    _ => return Err("only simple 'let name = ...' bindings are supported (no destructuring)".to_string()),
                },
                _ => return Err("only simple 'let name = ...' bindings are supported (no destructuring)".to_string()),
            };
            let init = local
                .init
                .as_ref()
                .ok_or_else(|| format!("'let {name}' needs an initializer — uninitialized bindings aren't supported"))?;
            if init.diverge.is_some() {
                return Err(format!("'let {name} = ... else {{ ... }}' isn't supported"));
            }
            let interned = intern(&name);
            if let Expr::If(if_expr) = &*init.expr {
                let stmts = lower_if_stmts(if_expr, ctx, Some(interned))?;
                // Var-backed, not inlined: see `Ctx::var_backed`'s doc for why
                // a branch-dependent value can't be a single inlined `SExpr`.
                ctx.var_backed.insert(name);
                Ok(stmts)
            } else {
                let expr = lower_expr(&init.expr, ctx)?;
                ctx.defs.insert(name, expr.clone());
                Ok(vec![SStmt::assign(interned, expr)])
            }
        }
        Stmt::Expr(expr, semi) => match expr {
            Expr::Macro(m) => lower_assert_stmts(&m.mac, ctx),
            // Block-like expressions (`if`/`else`) are valid Rust as a mid-
            // block statement with no trailing `;` — only the block's truly
            // *last* statement is tail position, and `lower_block` routes
            // that case to `lower_tail` before ever calling this function.
            Expr::If(if_expr) => lower_if_stmts(if_expr, ctx, None),
            _ if semi.is_some() => Err(
                "only 'assert!(...)'/'debug_assert!(...)' and 'if/else' statements are supported \
                 in a function body besides 'let' bindings (no bare expressions, loops, or \
                 function calls)"
                    .to_string(),
            ),
            _ => Err(
                "this expression can't appear without a trailing ';' here (only 'if/else', \
                 'assert!'/'debug_assert!', or the function's final tail expression can)"
                    .to_string(),
            ),
        },
        Stmt::Macro(stmt_mac) => lower_assert_stmts(&stmt_mac.mac, ctx),
        Stmt::Item(_) => Err("nested items (fn/struct/... inside a function body) aren't supported".to_string()),
    }
}

fn lower_assert_stmts(mac: &syn::Macro, ctx: &Ctx) -> Result<Vec<SStmt>, String> {
    let cond = lower_assert_macro(mac, ctx)?;
    Ok(vec![SStmt::assert(cond.clone()), SStmt::assume(cond)])
}

fn lower_assert_macro(mac: &syn::Macro, ctx: &Ctx) -> Result<SCond, String> {
    let name = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
    if name != "assert" && name != "debug_assert" {
        return Err(format!("macro '{name}!' isn't supported (only assert!/debug_assert!)"));
    }
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let args = parser
        .parse2(mac.tokens.clone())
        .map_err(|e| format!("couldn't parse {name}! arguments: {e}"))?;
    let cond_expr = args
        .first()
        .ok_or_else(|| format!("{name}! needs a condition"))?;
    lower_cond(cond_expr, ctx)
}

fn lower_expr(expr: &Expr, ctx: &Ctx) -> Result<SExpr, String> {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Int(li) => li
                .base10_parse::<i64>()
                .map(SExpr::const_)
                .map_err(|_| format!("integer literal '{li}' doesn't fit in i64")),
            other => Err(format!("unsupported literal '{other:?}' — only integer literals are supported")),
        },
        Expr::Path(p) => {
            let name = p
                .path
                .get_ident()
                .ok_or_else(|| "unsupported path expression (only plain identifiers are supported)".to_string())?
                .to_string();
            resolve_ident(&name, ctx)
        }
        Expr::Paren(inner) => lower_expr(&inner.expr, ctx),
        Expr::Group(inner) => lower_expr(&inner.expr, ctx),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => Ok(SExpr::sub(SExpr::const_(0), lower_expr(&u.expr, ctx)?)),
        Expr::Binary(b) => {
            let l = lower_expr(&b.left, ctx)?;
            let r = lower_expr(&b.right, ctx)?;
            match b.op {
                BinOp::Add(_) => Ok(SExpr::add(l, r)),
                BinOp::Sub(_) => Ok(SExpr::sub(l, r)),
                BinOp::Mul(_) => Ok(SExpr::mul(l, r)),
                BinOp::Div(_) => Ok(SExpr::div(l, r)),
                other => Err(format!("unsupported operator '{other:?}' in an arithmetic expression (only + - * / are supported)")),
            }
        }
        other => Err(format!(
            "unsupported expression ({}) — only integer literals, identifiers, '+ - * /', unary '-', and parens are supported",
            describe_syn_expr_kind(other)
        )),
    }
}

fn lower_cond(expr: &Expr, ctx: &Ctx) -> Result<SCond, String> {
    match expr {
        Expr::Paren(inner) => lower_cond(&inner.expr, ctx),
        Expr::Group(inner) => lower_cond(&inner.expr, ctx),
        Expr::Unary(u) if matches!(u.op, UnOp::Not(_)) => Ok(SCond::not(lower_cond(&u.expr, ctx)?)),
        Expr::Binary(b) => match b.op {
            BinOp::And(_) => Ok(SCond::and(lower_cond(&b.left, ctx)?, lower_cond(&b.right, ctx)?)),
            BinOp::Or(_) => Ok(SCond::or(lower_cond(&b.left, ctx)?, lower_cond(&b.right, ctx)?)),
            BinOp::Eq(_) => Ok(SCond::eq(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?)),
            BinOp::Ne(_) => Ok(SCond::not(SCond::eq(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?))),
            BinOp::Lt(_) => Ok(SCond::lt(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?)),
            BinOp::Le(_) => Ok(SCond::le(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?)),
            BinOp::Gt(_) => Ok(SCond::gt(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?)),
            BinOp::Ge(_) => Ok(SCond::ge(lower_cond_operand(&b.left, ctx)?, lower_cond_operand(&b.right, ctx)?)),
            other => Err(format!("unsupported operator '{other:?}' in a condition")),
        },
        other => Err(format!(
            "unsupported condition ({}) — only comparisons ('== != < <= > >='), '&& || !', and parens are supported",
            describe_syn_expr_kind(other)
        )),
    }
}

/// Lowers one side of a comparison, then rejects it if it transitively
/// refers to an `if`/`else`-produced (`var_backed`) name — see
/// `Ctx::var_backed`'s doc for why that specific combination isn't sound to
/// check here yet, rather than silently checking it against a free,
/// disconnected symbol.
fn lower_cond_operand(expr: &Expr, ctx: &Ctx) -> Result<SExpr, String> {
    let value = lower_expr(expr, ctx)?;
    if let Some(name) = expr_has_var(&value) {
        return Err(format!(
            "'{name}' was assigned inside an if/else branch, so its value can differ per path — \
             using it in a condition (assert!/if) isn't supported yet; it can still be used in \
             further arithmetic"
        ));
    }
    Ok(value)
}

/// True if `e` transitively contains an `SExpr::Var` (an if/else-produced,
/// `var_backed` name) — see `lower_cond_operand`.
fn expr_has_var(e: &SExpr) -> Option<&str> {
    match e {
        SExpr::Var(v) => Some(v.as_str()),
        SExpr::Const(_) | SExpr::Sym(_) => None,
        SExpr::Add(a, b) | SExpr::Sub(a, b) | SExpr::Mul(a, b) | SExpr::Div(a, b) => expr_has_var(a).or_else(|| expr_has_var(b)),
        SExpr::Neg(a) => expr_has_var(a),
    }
}

fn resolve_ident(name: &str, ctx: &Ctx) -> Result<SExpr, String> {
    if ctx.declared.contains(name) {
        Ok(SExpr::sym(name))
    } else if ctx.var_backed.contains(name) {
        Ok(SExpr::var(name))
    } else if let Some(def) = ctx.defs.get(name) {
        Ok(def.clone())
    } else {
        Err(format!("undeclared identifier '{name}' (not a parameter or a previous 'let' binding)"))
    }
}

fn describe_syn_expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Call(_) => "a function call",
        Expr::MethodCall(_) => "a method call",
        Expr::If(_) => "an 'if' expression",
        Expr::Loop(_) | Expr::While(_) | Expr::ForLoop(_) => "a loop",
        Expr::Field(_) => "field access",
        Expr::Index(_) => "indexing",
        Expr::Array(_) => "an array literal",
        Expr::Reference(_) => "a reference",
        Expr::Cast(_) => "a type cast",
        Expr::Struct(_) => "a struct literal",
        Expr::Macro(_) => "a macro call",
        _ => "an unrecognized expression form",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_a_real_rust_function() {
        let reports = analyze_rust_source(
            "fn divide(x: i64) -> i64 {\n    let z = 10 / x;\n    z\n}\n",
        )
        .unwrap();
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.name, "divide");
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert!(result.findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero));
    }

    #[test]
    fn assert_guard_flags_the_precondition_not_the_division() {
        // `assert!(x != 0)` is itself reachably violable — a caller really
        // can pass `x = 0` and hit the panic — so the tool correctly
        // reports *that* as a finding (a real, actionable one: this
        // function can panic). What it should NOT also report is a
        // division-by-zero on `10 / x`, since — given the assert didn't
        // panic — x != 0 is known for everything after it. Distinguishing
        // these two is the actual point of narrowing the path condition
        // through `assume` after a passed assertion.
        let reports = analyze_rust_source(
            "fn divide(x: i64) -> i64 {\n    assert!(x != 0);\n    10 / x\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert!(
            !result.findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero),
            "division should be proven safe given the assert held: {:?}",
            result.findings
        );
        assert!(
            result.findings.iter().any(|f| f.kind == crate::FindingKind::Assertion),
            "the precondition itself should be flagged as reachably violable: {:?}",
            result.findings
        );
    }

    #[test]
    fn guarded_division_with_explicit_let_is_safe_from_div_by_zero() {
        let reports = analyze_rust_source(
            "fn divide(x: i64) -> i64 {\n    assert!(x != 0);\n    let z = 10 / x;\n    z\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        assert!(!r
            .result
            .as_ref()
            .unwrap()
            .findings
            .iter()
            .any(|f| f.kind == crate::FindingKind::DivByZero));
    }

    #[test]
    fn multiple_functions_analyzed_independently() {
        let reports = analyze_rust_source(
            "fn a(x: i64) -> i64 { let z = 10 / x; z }\n\
             fn b(y: i64) -> i64 { assert!(y != 0); let z = 10 / y; z }\n",
        )
        .unwrap();
        assert_eq!(reports.len(), 2);
        assert!(reports[0].result.as_ref().unwrap().findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero));
        assert!(!reports[1]
            .result
            .as_ref()
            .unwrap()
            .findings
            .iter()
            .any(|f| f.kind == crate::FindingKind::DivByZero));
    }

    #[test]
    fn unsupported_construct_is_reported_not_silently_skipped() {
        // A loop is still outside the supported subset (unlike `if`, which
        // this crate now lowers) — this keeps coverage of the "explicit
        // error, never silently skipped" contract.
        let reports = analyze_rust_source(
            "fn f(x: i64) -> i64 {\n    let mut y = x;\n    loop { y = y + 1; }\n}\n",
        )
        .unwrap();
        assert!(reports[0].error.is_some());
        assert!(reports[0].result.is_none());
    }

    #[test]
    fn non_integer_param_is_reported() {
        let reports = analyze_rust_source("fn f(s: &str) { }\n").unwrap();
        assert!(reports[0].error.is_some());
    }

    #[test]
    fn if_else_tail_expression_explores_both_branches() {
        // `x > 0` is free, so both branches are feasible; neither divides
        // by anything, so this should be entirely clean, but it must
        // actually *analyze* (this exact function used to be the
        // "unsupported construct" example before if/else support existed).
        let reports = analyze_rust_source("fn f(x: i64) -> i64 {\n    if x > 0 { x } else { 0 }\n}\n").unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert!(result.is_clean(), "expected clean, got {:?}", result.findings);
        assert_eq!(result.paths_explored, 2, "both branches should be explored");
    }

    #[test]
    fn division_only_in_one_branch_is_flagged() {
        let reports = analyze_rust_source(
            "fn f(x: i64, y: i64) -> i64 {\n    if x > 0 {\n        10 / y\n    } else {\n        0\n    }\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert!(result.findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero));
    }

    #[test]
    fn if_else_as_let_value_feeds_further_arithmetic() {
        // `z`'s value differs per branch (var-backed, not inlined) but is
        // still usable in ordinary arithmetic afterward — `z / w` divides
        // by a free `w`, so that division-by-zero must still be found on
        // both paths through the preceding `if`.
        let reports = analyze_rust_source(
            "fn f(x: i64, w: i64) -> i64 {\n    let z = if x > 0 { 1 } else { 2 };\n    z / w\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert!(result.findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero));
    }

    #[test]
    fn if_without_else_cannot_be_used_as_a_value() {
        let reports = analyze_rust_source("fn f(x: i64) -> i64 {\n    let z = if x > 0 { 1 };\n    z\n}\n").unwrap();
        assert!(reports[0].error.is_some());
    }

    #[test]
    fn branch_dependent_value_in_later_condition_is_an_explicit_error() {
        // `z` differs per branch (var-backed) — using it in a later
        // `assert!` condition would need the condition to reason about a
        // branch-dependent value, which the frontend explicitly rejects
        // rather than silently checking it as an unconstrained free symbol.
        let reports = analyze_rust_source(
            "fn f(x: i64) -> i64 {\n    let z = if x > 0 { 1 } else { 2 };\n    assert!(z == 1);\n    z\n}\n",
        )
        .unwrap();
        assert!(reports[0].error.is_some());
    }

    #[test]
    fn else_if_chain_explores_every_branch() {
        let reports = analyze_rust_source(
            "fn f(x: i64) -> i64 {\n    if x > 10 {\n        1\n    } else if x > 0 {\n        2\n    } else {\n        3\n    }\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        assert_eq!(result.paths_explored, 3);
    }

    #[test]
    fn if_as_statement_with_no_value_still_checks_each_branch() {
        let reports = analyze_rust_source(
            "fn f(x: i64, y: i64) -> i64 {\n    if x > 0 {\n        assert!(y != 0);\n    }\n    10 / y\n}\n",
        )
        .unwrap();
        let r = &reports[0];
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        let result = r.result.as_ref().unwrap();
        // Division-by-zero is only proven safe on the branch where the
        // guard ran; the `x <= 0` branch skips the assert entirely, so a
        // reachable division by zero on `y` must still be reported.
        assert!(result.findings.iter().any(|f| f.kind == crate::FindingKind::DivByZero));
    }
}
