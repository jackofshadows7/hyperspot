pub mod ast {
    use bigdecimal::BigDecimal;
    use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
    use uuid::Uuid;

    #[derive(Clone, Debug)]
    pub enum Expr {
        And(Box<Expr>, Box<Expr>),
        Or(Box<Expr>, Box<Expr>),
        Not(Box<Expr>),
        Compare(Box<Expr>, CompareOperator, Box<Expr>),
        In(Box<Expr>, Vec<Expr>),
        Function(String, Vec<Expr>),
        Identifier(String),
        Value(Value),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CompareOperator {
        Eq,
        Ne,
        Gt,
        Ge,
        Lt,
        Le,
    }

    #[derive(Clone, Debug)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(BigDecimal),
        Uuid(Uuid),
        DateTime(DateTime<Utc>),
        Date(NaiveDate),
        Time(NaiveTime),
        String(String),
    }
}

#[derive(Clone, Debug, Default)]
pub struct ODataQuery(pub Option<Box<ast::Expr>>);

impl ODataQuery {
    pub fn none() -> Self {
        Self(None)
    }
    pub fn some(expr: ast::Expr) -> Self {
        Self(Some(Box::new(expr)))
    }
    pub fn as_ast(&self) -> Option<&ast::Expr> {
        self.0.as_deref()
    }
    pub fn into_ast(self) -> Option<ast::Expr> {
        self.0.map(|b| *b)
    }
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
}

impl From<Option<ast::Expr>> for ODataQuery {
    fn from(opt: Option<ast::Expr>) -> Self {
        match opt {
            Some(e) => ODataQuery::some(e),
            None => ODataQuery::none(),
        }
    }
}

#[cfg(feature = "with-odata-params")]
mod convert_odata_params {
    use super::ast::*;
    use odata_params::filters as od;

    impl From<od::CompareOperator> for CompareOperator {
        fn from(op: od::CompareOperator) -> Self {
            use od::CompareOperator::*;
            match op {
                Equal => CompareOperator::Eq,
                NotEqual => CompareOperator::Ne,
                GreaterThan => CompareOperator::Gt,
                GreaterOrEqual => CompareOperator::Ge,
                LessThan => CompareOperator::Lt,
                LessOrEqual => CompareOperator::Le,
            }
        }
    }

    impl From<od::Value> for Value {
        fn from(v: od::Value) -> Self {
            match v {
                od::Value::Null => Value::Null,
                od::Value::Bool(b) => Value::Bool(b),
                od::Value::Number(n) => Value::Number(n),
                od::Value::Uuid(u) => Value::Uuid(u),
                od::Value::DateTime(dt) => Value::DateTime(dt),
                od::Value::Date(d) => Value::Date(d),
                od::Value::Time(t) => Value::Time(t),
                od::Value::String(s) => Value::String(s),
            }
        }
    }

    impl From<od::Expr> for Expr {
        fn from(e: od::Expr) -> Self {
            use od::Expr::*;
            match e {
                And(a, b) => Expr::And(Box::new((*a).into()), Box::new((*b).into())),
                Or(a, b) => Expr::Or(Box::new((*a).into()), Box::new((*b).into())),
                Not(x) => Expr::Not(Box::new((*x).into())),
                Compare(l, op, r) => {
                    Expr::Compare(Box::new((*l).into()), op.into(), Box::new((*r).into()))
                }
                In(l, list) => Expr::In(
                    Box::new((*l).into()),
                    list.into_iter().map(|x| x.into()).collect(),
                ),
                Function(n, args) => {
                    Expr::Function(n, args.into_iter().map(|x| x.into()).collect())
                }
                Identifier(s) => Expr::Identifier(s),
                Value(v) => Expr::Value(v.into()),
            }
        }
    }
}
