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
//! Anything outside this subset (loops, `if`, function calls, non-integer
//! types, struct/array/pointer access, ...) makes that *function* report an
//! error naming the unsupported construct — analysis never silently ignores
//! code it can't model, since a verification tool that does that is worse
//! than useless. Other functions in the same file are still analyzed.

use std::collections::{HashMap, HashSet};

use syn::parse::Parser;
use syn::{BinOp, Expr, FnArg, Item, Pat, ReturnType, Stmt, Type, UnOp};
use tpt_for_symbolic_exec::{SCond, SExpr, SStmt};

use crate::{analyze_stmts, AnalysisResult};

/// The result of analyzing one `fn` item found in the source.
pub struct FunctionReport {
    pub name: String,
    pub params: Vec<String>,
    /// Set when this function's body uses a construct outside the
    /// supported subset (see the module doc). Nothing else is populated.
    pub error: Option<String>,
    pub result: Option<AnalysisResult>,
}

struct Ctx {
    declared: HashSet<String>,
    defs: HashMap<String, SExpr>,
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

    if !matches!(f.sig.output, ReturnType::Default) {
        // A return value isn't modeled (there's nothing yet to assert about
        // it) — this is a soft limitation, not a hard error, so keep going;
        // just note it once via the params list rather than failing.
    }

    let mut stmts: Vec<SStmt> = Vec::new();
    for stmt in &f.block.stmts {
        match lower_stmt(stmt, &mut ctx) {
            Ok(mut new_stmts) => stmts.append(&mut new_stmts),
            Err(msg) => return unsupported(name, params, msg),
        }
    }

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

/// Lowers one statement to zero or more `SStmt`s. An `assert!` lowers to
/// *two* statements — `Assert` (report a finding if it's reachably false)
/// followed by `Assume` (narrow the path condition, since real Rust
/// semantics guarantee the condition holds for everything after an
/// assertion that didn't panic) — because `tpt-for-symbolic-exec`'s
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
            let expr = lower_expr(&init.expr, ctx)?;
            let leaked: &'static str = Box::leak(name.clone().into_boxed_str());
            ctx.defs.insert(name, expr.clone());
            Ok(vec![SStmt::assign(leaked, expr)])
        }
        Stmt::Expr(expr, semi) => {
            if semi.is_some() {
                // A real statement (ends with `;`) — must be an assertion;
                // any other side-effecting bare expression isn't modeled.
                match expr {
                    Expr::Macro(m) => lower_assert_stmts(&m.mac, ctx),
                    _ => Err(
                        "only 'assert!(...)'/'debug_assert!(...)' statements are supported in a \
                         function body besides 'let' bindings (no bare expressions, if/loops, or \
                         function calls)"
                            .to_string(),
                    ),
                }
            } else {
                // No trailing `;` — this is the function's implicit return
                // value (always the block's last statement in valid Rust).
                // The return value isn't modeled as anything checkable yet,
                // but it must still be *within the supported subset* — a
                // function ending in e.g. a call we can't lower should say
                // so, not be silently accepted as if it verified fine.
                if let Expr::Macro(m) = expr {
                    // A trailing `assert!(...)` with no semicolon is valid
                    // Rust (assert! expands to `()`) — still an assertion.
                    lower_assert_stmts(&m.mac, ctx)
                } else {
                    // Lower into a synthetic assignment (never referenced
                    // again) rather than discarding it — an `Assign` is
                    // what actually triggers this engine's division-by-zero
                    // check; a bare `lower_expr` call with the result
                    // thrown away would silently skip checking any division
                    // in the return value, which is exactly the kind of
                    // soundness hole a verification tool can't have.
                    let value = lower_expr(expr, ctx)?;
                    let leaked: &'static str = Box::leak("__return".to_string().into_boxed_str());
                    Ok(vec![SStmt::assign(leaked, value)])
                }
            }
        }
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
            BinOp::Eq(_) => Ok(SCond::eq(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?)),
            BinOp::Ne(_) => Ok(SCond::not(SCond::eq(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?))),
            BinOp::Lt(_) => Ok(SCond::lt(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?)),
            BinOp::Le(_) => Ok(SCond::le(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?)),
            BinOp::Gt(_) => Ok(SCond::gt(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?)),
            BinOp::Ge(_) => Ok(SCond::ge(lower_expr(&b.left, ctx)?, lower_expr(&b.right, ctx)?)),
            other => Err(format!("unsupported operator '{other:?}' in a condition")),
        },
        other => Err(format!(
            "unsupported condition ({}) — only comparisons ('== != < <= > >='), '&& || !', and parens are supported",
            describe_syn_expr_kind(other)
        )),
    }
}

fn resolve_ident(name: &str, ctx: &Ctx) -> Result<SExpr, String> {
    if ctx.declared.contains(name) {
        Ok(SExpr::sym(name))
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
        let reports = analyze_rust_source(
            "fn f(x: i64) -> i64 {\n    if x > 0 { x } else { 0 }\n}\n",
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
}
