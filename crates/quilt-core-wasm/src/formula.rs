//! # formula.rs (wasm tier)
//!
//! A dependency-free expression evaluator for `formula` cells.
//!
//! ## Role in the system
//!
//! The native tier evaluates formulas with rhai (`packages/core/src/
//! cells/formula.rs`). rhai cannot compile for `wasm32-unknown-unknown`
//! (its `ahash` dependency needs OS randomness), so the wasm tier brings
//! its own evaluator. It covers the portable surface the golden contract
//! pins down:
//!
//! - literals: numbers, `true` / `false`
//! - cell references by bare dotted id (`bilge.level`, `compass.heading`)
//! - arithmetic `+ - * / %`, unary `-`
//! - comparisons `== != > >= < <=`
//! - boolean `&& || !`
//! - parentheses, and the helper functions `clamp`, `min`, `max`, `abs`
//!   (the same helpers the native tier registers into rhai)
//!
//! Evaluation is pure: given the same environment (a map of cell id →
//! JSON value) the result is deterministic. Dependencies are collected
//! from the parsed AST, which is what auto-detects the graph edges.
//!
//! Known gaps versus rhai (deliberate, documented in docs/wasm-target.md):
//! string literals, ternary, maps/arrays, script statements.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    AndAnd,
    OrOr,
    Bang,
}

fn tokenize(src: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Exponent: 1e-3, 2.5E+2
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            let n: f64 = text
                .parse()
                .map_err(|_| Error::FormulaParse(format!("bad number literal '{text}'")))?;
            toks.push(Tok::Num(n));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            // Dots are part of an identifier so `bilge.level` is one
            // token (cell ids are dotted).
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            toks.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let two = if i + 1 < chars.len() {
            Some([c, chars[i + 1]])
        } else {
            None
        };
        match two {
            Some(['=', '=']) => {
                toks.push(Tok::Eq);
                i += 2;
                continue;
            }
            Some(['!', '=']) => {
                toks.push(Tok::Ne);
                i += 2;
                continue;
            }
            Some(['>', '=']) => {
                toks.push(Tok::Ge);
                i += 2;
                continue;
            }
            Some(['<', '=']) => {
                toks.push(Tok::Le);
                i += 2;
                continue;
            }
            Some(['&', '&']) => {
                toks.push(Tok::AndAnd);
                i += 2;
                continue;
            }
            Some(['|', '|']) => {
                toks.push(Tok::OrOr);
                i += 2;
                continue;
            }
            _ => {}
        }
        let single = match c {
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            ',' => Some(Tok::Comma),
            '+' => Some(Tok::Plus),
            '-' => Some(Tok::Minus),
            '*' => Some(Tok::Star),
            '/' => Some(Tok::Slash),
            '%' => Some(Tok::Percent),
            '>' => Some(Tok::Gt),
            '<' => Some(Tok::Lt),
            '!' => Some(Tok::Bang),
            _ => None,
        };
        match single {
            Some(t) => {
                toks.push(t);
                i += 1;
            }
            None => {
                return Err(Error::FormulaParse(format!(
                    "unexpected character '{c}' at offset {i}"
                )))
            }
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Bool(bool),
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    And,
    Or,
}

// ---------------------------------------------------------------------------
// Parser (recursive descent, lowest precedence first)
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, want: &Tok, what: &str) -> Result<Tok> {
        match self.next() {
            Some(t) if &t == want => Ok(t),
            Some(t) => Err(Error::FormulaParse(format!("expected {what}, found {t:?}"))),
            None => Err(Error::FormulaParse(format!(
                "expected {what}, found end of expression"
            ))),
        }
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_and()?;
        while self.peek() == Some(&Tok::OrOr) {
            self.next();
            let rhs = self.parse_and()?;
            lhs = Expr::Binary(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_cmp()?;
        while self.peek() == Some(&Tok::AndAnd) {
            self.next();
            let rhs = self.parse_cmp()?;
            lhs = Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr> {
        let lhs = self.parse_add()?;
        let op = match self.peek() {
            Some(Tok::Eq) => Some(BinOp::Eq),
            Some(Tok::Ne) => Some(BinOp::Ne),
            Some(Tok::Gt) => Some(BinOp::Gt),
            Some(Tok::Ge) => Some(BinOp::Ge),
            Some(Tok::Lt) => Some(BinOp::Lt),
            Some(Tok::Le) => Some(BinOp::Le),
            _ => None,
        };
        match op {
            Some(op) => {
                self.next();
                let rhs = self.parse_add()?;
                Ok(Expr::Binary(op, Box::new(lhs), Box::new(rhs)))
            }
            None => Ok(lhs),
        }
    }

    fn parse_add(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => Some(BinOp::Add),
                Some(Tok::Minus) => Some(BinOp::Sub),
                _ => None,
            };
            match op {
                Some(op) => {
                    self.next();
                    let rhs = self.parse_mul()?;
                    lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
                }
                None => return Ok(lhs),
            }
        }
    }

    fn parse_mul(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => Some(BinOp::Mul),
                Some(Tok::Slash) => Some(BinOp::Div),
                Some(Tok::Percent) => Some(BinOp::Mod),
                _ => None,
            };
            match op {
                Some(op) => {
                    self.next();
                    let rhs = self.parse_unary()?;
                    lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
                }
                None => return Ok(lhs),
            }
        }
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.next();
                let inner = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(inner)))
            }
            Some(Tok::Bang) => {
                self.next();
                let inner = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(inner)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Ident(name)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.next();
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.parse_or()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Tok::RParen, "')' after function arguments")?;
                    Ok(Expr::Call(name, args))
                } else {
                    match name.as_str() {
                        "true" => Ok(Expr::Bool(true)),
                        "false" => Ok(Expr::Bool(false)),
                        _ => Ok(Expr::Ident(name)),
                    }
                }
            }
            Some(Tok::LParen) => {
                let inner = self.parse_or()?;
                self.expect(&Tok::RParen, "')' after parenthesized expression")?;
                Ok(inner)
            }
            Some(t) => Err(Error::FormulaParse(format!("unexpected token {t:?}"))),
            None => Err(Error::FormulaParse("unexpected end of expression".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A formula value: a number or a boolean. This is the portable subset
/// of `serde_json::Value` formulas produce and consume.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Val {
    Num(f64),
    Bool(bool),
}

fn json_to_val(id: &str, v: &Value) -> Result<Val> {
    match v {
        Value::Number(n) => n
            .as_f64()
            .map(Val::Num)
            .ok_or_else(|| Error::FormulaEval(format!("cell '{id}': non-finite number"))),
        Value::Bool(b) => Ok(Val::Bool(*b)),
        other => Err(Error::FormulaEval(format!(
            "cell '{id}': {other} is not a number or bool (unsupported in wasm formulas)"
        ))),
    }
}

fn val_to_json(v: Val) -> Result<Value> {
    match v {
        Val::Bool(b) => Ok(Value::Bool(b)),
        Val::Num(n) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .ok_or_else(|| Error::FormulaEval("result is NaN or infinite".into())),
    }
}

// ---------------------------------------------------------------------------
// The formula
// ---------------------------------------------------------------------------

/// A compiled formula: source text plus parsed AST. Compile once,
/// evaluate many times with different cell-value environments.
#[derive(Debug, Clone)]
pub struct Formula {
    /// The original source (with any leading `=` stripped).
    pub source: String,
    /// The parsed expression.
    ast: Expr,
}

impl Formula {
    /// Compile a formula expression. A leading `=` (the sheet-DSL
    /// convention) is stripped if present.
    pub fn compile(source: &str) -> Result<Self> {
        let body = source.strip_prefix('=').unwrap_or(source).trim();
        let toks = tokenize(body)?;
        if toks.is_empty() {
            return Err(Error::FormulaParse("empty expression".into()));
        }
        let mut parser = Parser { toks, pos: 0 };
        let ast = parser.parse_or()?;
        if parser.pos != parser.toks.len() {
            return Err(Error::FormulaParse(format!(
                "trailing tokens at position {}",
                parser.pos
            )));
        }
        Ok(Self {
            source: body.to_string(),
            ast,
        })
    }

    /// Evaluate against an environment of cell values. Every free
    /// identifier must be present in `env` (or be `true`/`false`,
    /// handled at parse time).
    pub fn eval(&self, env: &BTreeMap<String, Value>) -> Result<Value> {
        let v = self.eval_expr(&self.ast, env)?;
        val_to_json(v)
    }

    /// All identifiers referenced by the expression (superset of the
    /// real dependencies until intersected with known cell ids).
    pub fn dependencies(&self) -> Vec<String> {
        let mut ids = BTreeSet::new();
        self.collect_idents(&self.ast, &mut ids);
        ids.into_iter().collect()
    }

    fn collect_idents(&self, e: &Expr, out: &mut BTreeSet<String>) {
        match e {
            Expr::Ident(id) => {
                out.insert(id.clone());
            }
            Expr::Unary(_, inner) => self.collect_idents(inner, out),
            Expr::Binary(_, l, r) => {
                self.collect_idents(l, out);
                self.collect_idents(r, out);
            }
            Expr::Call(_, args) => {
                for a in args {
                    self.collect_idents(a, out);
                }
            }
            Expr::Num(_) | Expr::Bool(_) => {}
        }
    }

    fn eval_expr(&self, e: &Expr, env: &BTreeMap<String, Value>) -> Result<Val> {
        match e {
            Expr::Num(n) => Ok(Val::Num(*n)),
            Expr::Bool(b) => Ok(Val::Bool(*b)),
            Expr::Ident(id) => match env.get(id) {
                Some(v) => json_to_val(id, v),
                None => Err(Error::FormulaEval(format!(
                    "unknown cell reference '{id}' (not in dependency snapshot)"
                ))),
            },
            Expr::Unary(op, inner) => {
                let v = self.eval_expr(inner, env)?;
                match (op, v) {
                    (UnOp::Neg, Val::Num(n)) => Ok(Val::Num(-n)),
                    (UnOp::Not, Val::Bool(b)) => Ok(Val::Bool(!b)),
                    (op, v) => Err(Error::FormulaEval(format!("cannot apply {op:?} to {v:?}"))),
                }
            }
            Expr::Binary(op, l, r) => {
                let lv = self.eval_expr(l, env)?;
                let rv = self.eval_expr(r, env)?;
                match (op, lv, rv) {
                    (BinOp::Add, Val::Num(a), Val::Num(b)) => Ok(Val::Num(a + b)),
                    (BinOp::Sub, Val::Num(a), Val::Num(b)) => Ok(Val::Num(a - b)),
                    (BinOp::Mul, Val::Num(a), Val::Num(b)) => Ok(Val::Num(a * b)),
                    (BinOp::Div, Val::Num(a), Val::Num(b)) if b != 0.0 => Ok(Val::Num(a / b)),
                    (BinOp::Mod, Val::Num(a), Val::Num(b)) if b != 0.0 => Ok(Val::Num(a % b)),
                    (BinOp::Eq, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a == b)),
                    (BinOp::Eq, Val::Bool(a), Val::Bool(b)) => Ok(Val::Bool(a == b)),
                    (BinOp::Ne, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a != b)),
                    (BinOp::Ne, Val::Bool(a), Val::Bool(b)) => Ok(Val::Bool(a != b)),
                    (BinOp::Gt, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a > b)),
                    (BinOp::Ge, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a >= b)),
                    (BinOp::Lt, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a < b)),
                    (BinOp::Le, Val::Num(a), Val::Num(b)) => Ok(Val::Bool(a <= b)),
                    (BinOp::And, Val::Bool(a), Val::Bool(b)) => Ok(Val::Bool(a && b)),
                    (BinOp::Or, Val::Bool(a), Val::Bool(b)) => Ok(Val::Bool(a || b)),
                    (op, lv, rv) => Err(Error::FormulaEval(format!(
                        "cannot apply {op:?} to {lv:?} and {rv:?}"
                    ))),
                }
            }
            Expr::Call(name, args) => self.eval_call(name, args, env),
        }
    }

    fn eval_call(&self, name: &str, args: &[Expr], env: &BTreeMap<String, Value>) -> Result<Val> {
        // The helper set the native tier registers into rhai, plus the
        // number-argument builtins rhai carries by default.
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval_expr(a, env)?);
        }
        let all_nums = |vals: &Vec<Val>| -> Result<Vec<f64>> {
            vals.iter()
                .map(|v| match v {
                    Val::Num(n) => Ok(*n),
                    Val::Bool(_) => Err(Error::FormulaEval(format!(
                        "function '{name}' expects numbers, got a bool"
                    ))),
                })
                .collect()
        };
        match name {
            "clamp" if vals.len() == 3 => {
                let xs = all_nums(&vals)?;
                Ok(Val::Num(xs[0].max(xs[1]).min(xs[2])))
            }
            "abs" if vals.len() == 1 => {
                let xs = all_nums(&vals)?;
                Ok(Val::Num(xs[0].abs()))
            }
            "min" if !vals.is_empty() => {
                let xs = all_nums(&vals)?;
                Ok(Val::Num(xs.iter().cloned().fold(f64::INFINITY, f64::min)))
            }
            "max" if !vals.is_empty() => {
                let xs = all_nums(&vals)?;
                Ok(Val::Num(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)))
            }
            _ => Err(Error::FormulaEval(format!(
                "unknown function '{name}' with {} argument(s) (wasm helpers: clamp, abs, min, max)",
                vals.len()
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn golden_formula_shapes_evaluate() {
        // pump.should_run, initial: 40 >= 80 -> false
        let f = Formula::compile("=bilge.level >= bilge.threshold").unwrap();
        let e = env(&[
            ("bilge.level", json!(40.0)),
            ("bilge.threshold", json!(80.0)),
        ]);
        assert_eq!(f.eval(&e).unwrap(), json!(false));

        // pump.relay_cmd, initial: clamp((40-80)*0.5, -30, 30) -> -20
        let f =
            Formula::compile("=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)").unwrap();
        assert_eq!(f.eval(&e).unwrap(), json!(-20.0));

        // After pushing 85: 85 >= 80 -> true, clamp((85-80)*0.5) -> 2.5
        let e = env(&[
            ("bilge.level", json!(85.0)),
            ("bilge.threshold", json!(80.0)),
        ]);
        let f = Formula::compile("bilge.level >= bilge.threshold").unwrap();
        assert_eq!(f.eval(&e).unwrap(), json!(true));
        let f =
            Formula::compile("=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)").unwrap();
        assert_eq!(f.eval(&e).unwrap(), json!(2.5));
    }

    #[test]
    fn dependencies_are_dotted_identifiers() {
        let f =
            Formula::compile("=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)").unwrap();
        assert_eq!(
            f.dependencies(),
            vec!["bilge.level".to_string(), "bilge.threshold".to_string()]
        );
    }

    #[test]
    fn precedence_and_operators() {
        let e = env(&[]);
        assert_eq!(
            Formula::compile("1 + 2 * 3").unwrap().eval(&e).unwrap(),
            json!(7.0)
        );
        assert_eq!(
            Formula::compile("-(1 + 2) * 4").unwrap().eval(&e).unwrap(),
            json!(-12.0)
        );
        assert_eq!(
            Formula::compile("7 % 3").unwrap().eval(&e).unwrap(),
            json!(1.0)
        );
        assert_eq!(
            Formula::compile("1 < 2 && 3 >= 3 || false")
                .unwrap()
                .eval(&e)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            Formula::compile("!false == true")
                .unwrap()
                .eval(&e)
                .unwrap(),
            json!(true)
        );
        assert_eq!(
            Formula::compile("min(3.0, 1.0, 2.0)")
                .unwrap()
                .eval(&e)
                .unwrap(),
            json!(1.0)
        );
        assert_eq!(
            Formula::compile("max(-1.5)").unwrap().eval(&e).unwrap(),
            json!(-1.5)
        );
        assert_eq!(
            Formula::compile("abs(0.0 - 2.5)")
                .unwrap()
                .eval(&e)
                .unwrap(),
            json!(2.5)
        );
    }

    #[test]
    fn malformed_expressions_error() {
        assert!(Formula::compile("=1 +").is_err());
        assert!(Formula::compile("=(unclosed").is_err());
        assert!(Formula::compile("=2 $ 3").is_err());
        assert!(Formula::compile("=").is_err());
        assert!(Formula::compile("=1 2").is_err());
    }

    #[test]
    fn unknown_reference_and_bad_types_error() {
        let f = Formula::compile("=a + b").unwrap();
        let e = env(&[("a", json!(1.0))]);
        assert!(f.eval(&e).is_err());
        let e = env(&[("a", json!(1.0)), ("b", json!("text"))]);
        assert!(f.eval(&e).is_err());
    }
}
