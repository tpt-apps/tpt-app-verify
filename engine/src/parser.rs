//! A tiny line-based verification DSL, parsed into `tpt_for_symbolic_exec`'s
//! `SStmt`/`SExpr`/`SCond` AST.
//!
//! Grammar (one statement per line; `#` starts a line comment):
//!
//! ```text
//! input NAME              declares NAME as a free symbolic input
//! NAME = EXPR             assigns the result of EXPR to a local variable
//! assert COND              every feasible path must satisfy COND
//! assume COND              prune paths that don't satisfy COND
//! ```
//!
//! `EXPR` is standard arithmetic (`+ - * /`, unary `-`, parens, integers,
//! identifiers). `COND` is a comparison (`== != < <= > >=`), optionally
//! combined with `&&` / `||` and negated with `!`.
//!
//! An identifier resolves to a free symbolic input (`SExpr::sym`) if it was
//! declared with `input`, or — if it was previously assigned — to that
//! assignment's expression, inlined. Inlining (rather than emitting
//! `SExpr::Var`) is deliberate: `tpt-for-symbolic-exec`'s interpreter only
//! substitutes variable definitions through its store for `Assign`
//! right-hand sides, not for `Assert`/`Assume` conditions, so a condition
//! written in terms of a bare `Var` would be checked against an
//! unconstrained value instead of its actual definition. Inlining at parse
//! time keeps every condition expressed purely in terms of `Sym`/`Const`,
//! which both code paths handle correctly. An undeclared identifier fails
//! parsing with a clear error naming it and its line number.

use std::collections::{HashMap, HashSet};

use tpt_for_symbolic_exec::{SCond, SExpr, SStmt};

pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl ParseError {
    fn new(line: usize, message: impl Into<String>) -> ParseError {
        ParseError {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

struct Ctx {
    declared: HashSet<String>,
    defs: HashMap<String, SExpr>,
}

/// Parses `source` into a straight-line program (`Vec<SStmt>`).
pub fn parse(source: &str) -> Result<Vec<SStmt>, ParseError> {
    let mut ctx = Ctx {
        declared: HashSet::new(),
        defs: HashMap::new(),
    };
    let mut stmts = Vec::new();

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("input ") {
            let name = rest.trim();
            if name.is_empty() || !is_ident(name) {
                return Err(ParseError::new(line_no, format!("invalid input name '{name}'")));
            }
            ctx.declared.insert(name.to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("assert ") {
            let cond = parse_cond(rest.trim(), &ctx, line_no)?;
            stmts.push(SStmt::assert(cond));
            continue;
        }

        if let Some(rest) = line.strip_prefix("assume ") {
            let cond = parse_cond(rest.trim(), &ctx, line_no)?;
            stmts.push(SStmt::assume(cond));
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            // Guard against `==` being mistaken for an assignment.
            if line.as_bytes().get(eq_pos + 1) != Some(&b'=') {
                let name = line[..eq_pos].trim();
                let expr_src = line[eq_pos + 1..].trim();
                if name.is_empty() || !is_ident(name) {
                    return Err(ParseError::new(line_no, format!("invalid variable name '{name}'")));
                }
                let expr = parse_expr(expr_src, &ctx, line_no)?;
                let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
                stmts.push(SStmt::assign(leaked, expr.clone()));
                ctx.defs.insert(name.to_string(), expr);
                continue;
            }
        }

        return Err(ParseError::new(line_no, format!("couldn't parse '{line}'")));
    }

    Ok(stmts)
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---- Expression parsing (recursive descent over a pre-tokenized string) ----

struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Tokens<'a> {
    fn new(s: &'a str) -> Tokens<'a> {
        Tokens { rest: s.trim() }
    }

    fn skip_ws(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn eat_op(&mut self, op: &str) -> bool {
        self.skip_ws();
        if self.rest.starts_with(op) {
            self.rest = &self.rest[op.len()..];
            true
        } else {
            false
        }
    }

    fn next_token(&mut self) -> Option<String> {
        self.skip_ws();
        if self.rest.is_empty() {
            return None;
        }
        let bytes = self.rest.as_bytes();
        if bytes[0].is_ascii_digit() {
            let end = self.rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(self.rest.len());
            let tok = self.rest[..end].to_string();
            self.rest = &self.rest[end..];
            Some(tok)
        } else if bytes[0].is_ascii_alphabetic() || bytes[0] == b'_' {
            let end = self
                .rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(self.rest.len());
            let tok = self.rest[..end].to_string();
            self.rest = &self.rest[end..];
            Some(tok)
        } else {
            None
        }
    }
}

fn resolve_ident(name: &str, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    if ctx.declared.contains(name) {
        Ok(SExpr::sym(name))
    } else if let Some(def) = ctx.defs.get(name) {
        Ok(def.clone())
    } else {
        Err(ParseError::new(
            line_no,
            format!("undeclared identifier '{name}' (add 'input {name}' or assign it first)"),
        ))
    }
}

fn parse_expr(src: &str, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    let mut t = Tokens::new(src);
    let e = parse_add_sub(&mut t, ctx, line_no)?;
    t.skip_ws();
    if !t.rest.is_empty() {
        return Err(ParseError::new(line_no, format!("unexpected trailing input '{}'", t.rest)));
    }
    Ok(e)
}

fn parse_add_sub(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    let mut lhs = parse_mul_div(t, ctx, line_no)?;
    loop {
        if t.eat_op("+") {
            let rhs = parse_mul_div(t, ctx, line_no)?;
            lhs = SExpr::add(lhs, rhs);
        } else if t.eat_op("-") {
            let rhs = parse_mul_div(t, ctx, line_no)?;
            lhs = SExpr::sub(lhs, rhs);
        } else {
            break;
        }
    }
    Ok(lhs)
}

fn parse_mul_div(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    let mut lhs = parse_unary(t, ctx, line_no)?;
    loop {
        if t.eat_op("*") {
            let rhs = parse_unary(t, ctx, line_no)?;
            lhs = SExpr::mul(lhs, rhs);
        } else if t.eat_op("/") {
            let rhs = parse_unary(t, ctx, line_no)?;
            lhs = SExpr::div(lhs, rhs);
        } else {
            break;
        }
    }
    Ok(lhs)
}

fn parse_unary(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    if t.eat_op("-") {
        let inner = parse_unary(t, ctx, line_no)?;
        return Ok(SExpr::sub(SExpr::const_(0), inner));
    }
    parse_atom(t, ctx, line_no)
}

fn parse_atom(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SExpr, ParseError> {
    t.skip_ws();
    if t.eat_op("(") {
        let inner = parse_add_sub(t, ctx, line_no)?;
        if !t.eat_op(")") {
            return Err(ParseError::new(line_no, "expected ')'"));
        }
        return Ok(inner);
    }
    let tok = t
        .next_token()
        .ok_or_else(|| ParseError::new(line_no, "expected a number or identifier"))?;
    if tok.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        let n: i64 = tok
            .parse()
            .map_err(|_| ParseError::new(line_no, format!("invalid number '{tok}'")))?;
        Ok(SExpr::const_(n))
    } else {
        resolve_ident(&tok, ctx, line_no)
    }
}

// ---- Condition parsing ----

fn parse_cond(src: &str, ctx: &Ctx, line_no: usize) -> Result<SCond, ParseError> {
    let mut t = Tokens::new(src);
    let c = parse_or(&mut t, ctx, line_no)?;
    t.skip_ws();
    if !t.rest.is_empty() {
        return Err(ParseError::new(line_no, format!("unexpected trailing input '{}'", t.rest)));
    }
    Ok(c)
}

fn parse_or(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SCond, ParseError> {
    let mut lhs = parse_and(t, ctx, line_no)?;
    while t.eat_op("||") {
        let rhs = parse_and(t, ctx, line_no)?;
        lhs = SCond::or(lhs, rhs);
    }
    Ok(lhs)
}

fn parse_and(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SCond, ParseError> {
    let mut lhs = parse_not(t, ctx, line_no)?;
    while t.eat_op("&&") {
        let rhs = parse_not(t, ctx, line_no)?;
        lhs = SCond::and(lhs, rhs);
    }
    Ok(lhs)
}

fn parse_not(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SCond, ParseError> {
    if t.eat_op("!") {
        let inner = parse_not(t, ctx, line_no)?;
        return Ok(SCond::not(inner));
    }
    parse_cmp(t, ctx, line_no)
}

fn parse_cmp(t: &mut Tokens, ctx: &Ctx, line_no: usize) -> Result<SCond, ParseError> {
    let lhs = parse_add_sub(t, ctx, line_no)?;
    t.skip_ws();
    if t.eat_op("==") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::eq(lhs, rhs))
    } else if t.eat_op("!=") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::not(SCond::eq(lhs, rhs)))
    } else if t.eat_op("<=") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::le(lhs, rhs))
    } else if t.eat_op(">=") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::ge(lhs, rhs))
    } else if t.eat_op("<") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::lt(lhs, rhs))
    } else if t.eat_op(">") {
        let rhs = parse_add_sub(t, ctx, line_no)?;
        Ok(SCond::gt(lhs, rhs))
    } else {
        Err(ParseError::new(line_no, "expected a comparison operator (== != < <= > >=)"))
    }
}
