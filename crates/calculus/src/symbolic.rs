//! Lightweight symbolic expression evaluator for agent policy rules.
//!
//! Enables agents to evaluate conditional logic on telemetry signals without
//! requiring compiled code. Used by ThreatHunter for rule matching,
//! Guardian for policy enforcement, and the Learning agent for adaptation.
//!
//! Grammar:
//! ```text
//! expr := number | variable | unary_op expr | expr binary_op expr | "(" expr ")"
//! binary_op := "+" | "-" | "*" | "/" | ">" | "<" | ">=" | "<=" | "==" | "!=" | "&&" | "||"
//! unary_op := "-" | "!"
//! ```

use std::collections::HashMap;

use crate::error::{CalculusError, Result};

/// A symbolic expression node.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Literal numeric value.
    Literal(f64),
    /// Named variable (resolved from context at eval time).
    Variable(String),
    /// Unary operation.
    Unary(UnaryOp, Box<Expr>),
    /// Binary operation.
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Negate,
    Not,
    Abs,
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Gt,
    Lt,
    Gte,
    Lte,
    Eq,
    Neq,
    And,
    Or,
    Min,
    Max,
}

impl Expr {
    /// Evaluate the expression with the given variable bindings.
    pub fn eval(&self, ctx: &HashMap<String, f64>) -> Result<f64> {
        match self {
            Expr::Literal(v) => Ok(*v),
            Expr::Variable(name) => ctx
                .get(name)
                .copied()
                .ok_or_else(|| CalculusError::InvalidThreshold(format!("undefined variable: {}", name))),
            Expr::Unary(op, inner) => {
                let v = inner.eval(ctx)?;
                Ok(match op {
                    UnaryOp::Negate => -v,
                    UnaryOp::Not => {
                        if v == 0.0 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    UnaryOp::Abs => v.abs(),
                })
            }
            Expr::Binary(op, left, right) => {
                let l = left.eval(ctx)?;
                let r = right.eval(ctx)?;
                Ok(match op {
                    BinaryOp::Add => l + r,
                    BinaryOp::Sub => l - r,
                    BinaryOp::Mul => l * r,
                    BinaryOp::Div => {
                        if r == 0.0 {
                            return Err(CalculusError::DivisionByZero);
                        }
                        l / r
                    }
                    BinaryOp::Gt => bool_to_f64(l > r),
                    BinaryOp::Lt => bool_to_f64(l < r),
                    BinaryOp::Gte => bool_to_f64(l >= r),
                    BinaryOp::Lte => bool_to_f64(l <= r),
                    BinaryOp::Eq => bool_to_f64((l - r).abs() < f64::EPSILON),
                    BinaryOp::Neq => bool_to_f64((l - r).abs() >= f64::EPSILON),
                    BinaryOp::And => bool_to_f64(l != 0.0 && r != 0.0),
                    BinaryOp::Or => bool_to_f64(l != 0.0 || r != 0.0),
                    BinaryOp::Min => l.min(r),
                    BinaryOp::Max => l.max(r),
                })
            }
        }
    }

    /// Evaluate as boolean (non-zero = true).
    pub fn eval_bool(&self, ctx: &HashMap<String, f64>) -> Result<bool> {
        Ok(self.eval(ctx)? != 0.0)
    }
}

fn bool_to_f64(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

// --- Builder helpers for constructing expressions programmatically ---

/// Literal value.
pub fn lit(v: f64) -> Expr {
    Expr::Literal(v)
}

/// Variable reference.
pub fn var(name: &str) -> Expr {
    Expr::Variable(name.to_string())
}

/// Negate.
pub fn neg(e: Expr) -> Expr {
    Expr::Unary(UnaryOp::Negate, Box::new(e))
}

/// Absolute value.
pub fn abs(e: Expr) -> Expr {
    Expr::Unary(UnaryOp::Abs, Box::new(e))
}

/// Logical NOT.
pub fn not(e: Expr) -> Expr {
    Expr::Unary(UnaryOp::Not, Box::new(e))
}

/// Addition.
pub fn add(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Add, Box::new(l), Box::new(r))
}

/// Subtraction.
pub fn sub(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Sub, Box::new(l), Box::new(r))
}

/// Multiplication.
pub fn mul(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Mul, Box::new(l), Box::new(r))
}

/// Division.
pub fn div(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Div, Box::new(l), Box::new(r))
}

/// Greater than.
pub fn gt(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Gt, Box::new(l), Box::new(r))
}

/// Less than.
pub fn lt(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Lt, Box::new(l), Box::new(r))
}

/// Greater than or equal.
pub fn gte(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Gte, Box::new(l), Box::new(r))
}

/// Less than or equal.
pub fn lte(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Lte, Box::new(l), Box::new(r))
}

/// Logical AND.
pub fn and(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::And, Box::new(l), Box::new(r))
}

/// Logical OR.
pub fn or(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Or, Box::new(l), Box::new(r))
}

/// Minimum.
pub fn min(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Min, Box::new(l), Box::new(r))
}

/// Maximum.
pub fn max(l: Expr, r: Expr) -> Expr {
    Expr::Binary(BinaryOp::Max, Box::new(l), Box::new(r))
}

/// A named policy rule that evaluates a condition expression.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub name: String,
    pub condition: Expr,
    pub description: String,
}

impl PolicyRule {
    pub fn new(name: impl Into<String>, condition: Expr, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            condition,
            description: description.into(),
        }
    }

    /// Evaluate the policy rule. Returns true if the condition is met.
    pub fn evaluate(&self, ctx: &HashMap<String, f64>) -> Result<bool> {
        self.condition.eval_bool(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn literal_eval() {
        let e = lit(42.0);
        assert_eq!(e.eval(&HashMap::new()).unwrap(), 42.0);
    }

    #[test]
    fn variable_eval() {
        let e = var("x");
        let c = ctx(&[("x", 3.14)]);
        assert_eq!(e.eval(&c).unwrap(), 3.14);
    }

    #[test]
    fn undefined_variable() {
        let e = var("y");
        assert!(e.eval(&HashMap::new()).is_err());
    }

    #[test]
    fn arithmetic() {
        let c = ctx(&[("a", 10.0), ("b", 3.0)]);
        assert_eq!(add(var("a"), var("b")).eval(&c).unwrap(), 13.0);
        assert_eq!(sub(var("a"), var("b")).eval(&c).unwrap(), 7.0);
        assert_eq!(mul(var("a"), var("b")).eval(&c).unwrap(), 30.0);
        assert!((div(var("a"), var("b")).eval(&c).unwrap() - 3.333333).abs() < 0.001);
    }

    #[test]
    fn division_by_zero() {
        assert!(div(lit(1.0), lit(0.0)).eval(&HashMap::new()).is_err());
    }

    #[test]
    fn comparison() {
        let c = ctx(&[("x", 5.0)]);
        assert!(gt(var("x"), lit(3.0)).eval_bool(&c).unwrap());
        assert!(!lt(var("x"), lit(3.0)).eval_bool(&c).unwrap());
        assert!(gte(var("x"), lit(5.0)).eval_bool(&c).unwrap());
    }

    #[test]
    fn logical() {
        let c = ctx(&[("a", 1.0), ("b", 0.0)]);
        assert!(!and(var("a"), var("b")).eval_bool(&c).unwrap());
        assert!(or(var("a"), var("b")).eval_bool(&c).unwrap());
        assert!(not(var("b")).eval_bool(&c).unwrap());
    }

    #[test]
    fn threat_detection_rule() {
        // Rule: flag if z_score > 3.0 AND request_rate > 100
        let rule = PolicyRule::new(
            "anomaly_detect",
            and(gt(var("z_score"), lit(3.0)), gt(var("request_rate"), lit(100.0))),
            "flag anomalous traffic",
        );

        let normal = ctx(&[("z_score", 1.5), ("request_rate", 50.0)]);
        assert!(!rule.evaluate(&normal).unwrap());

        let anomalous = ctx(&[("z_score", 4.2), ("request_rate", 200.0)]);
        assert!(rule.evaluate(&anomalous).unwrap());
    }

    #[test]
    fn nested_expression() {
        // abs(x - mean) / stddev > 3.0
        let expr = gt(
            div(abs(sub(var("x"), var("mean"))), var("stddev")),
            lit(3.0),
        );
        let c = ctx(&[("x", 100.0), ("mean", 50.0), ("stddev", 10.0)]);
        // |100 - 50| / 10 = 5.0 > 3.0 → true
        assert!(expr.eval_bool(&c).unwrap());
    }

    #[test]
    fn min_max() {
        let c = ctx(&[("a", 5.0), ("b", 3.0)]);
        assert_eq!(min(var("a"), var("b")).eval(&c).unwrap(), 3.0);
        assert_eq!(max(var("a"), var("b")).eval(&c).unwrap(), 5.0);
    }
}
