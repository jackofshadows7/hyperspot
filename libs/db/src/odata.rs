//! OData (filters) → sea_orm::Condition compiler (AST in, SQL out).
//! Parsing belongs to API/ingress. This module only consumes `odata_core::ast::Expr`.

use std::collections::HashMap;

use bigdecimal::{BigDecimal, ToPrimitive};
use odata_core::{ast as core, ODataQuery};
use rust_decimal::Decimal;
use sea_orm::{sea_query::Expr, ColumnTrait, Condition, EntityTrait, QueryFilter};
use thiserror::Error;

/// Whitelisted field kind → used to coerce `core::Value` into `sea_orm::Value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    String,
    I64,
    F64,
    Bool,
    Uuid,
    DateTimeUtc,
    Date,
    Time,
    Decimal,
}

#[derive(Clone)]
pub struct Field<E: EntityTrait> {
    pub col: E::Column,
    pub kind: FieldKind,
}

#[derive(Clone)]
pub struct FieldMap<E: EntityTrait> {
    map: HashMap<String, Field<E>>,
}

impl<E: EntityTrait> Default for FieldMap<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: EntityTrait> FieldMap<E> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn insert(mut self, api_name: impl Into<String>, col: E::Column, kind: FieldKind) -> Self {
        self.map
            .insert(api_name.into().to_lowercase(), Field { col, kind });
        self
    }
    pub fn get(&self, name: &str) -> Option<&Field<E>> {
        self.map.get(&name.to_lowercase())
    }
}

#[derive(Debug, Error, Clone)]
pub enum ODataBuildError {
    #[error("unknown field: {0}")]
    UnknownField(String),

    #[error("type mismatch: expected {expected:?}, got {got}")]
    TypeMismatch {
        expected: FieldKind,
        got: &'static str,
    },

    #[error("unsupported operator: {0:?}")]
    UnsupportedOp(core::CompareOperator),

    #[error("unsupported function or args: {0}()")]
    UnsupportedFn(String),

    #[error("IN() list supports only literals")]
    NonLiteralInList,

    #[error("bare identifier not allowed: {0}")]
    BareIdentifier(String),

    #[error("bare literal not allowed")]
    BareLiteral,

    #[error("{0}")]
    Other(&'static str),
}
pub type ODataBuildResult<T> = Result<T, ODataBuildError>;

/* ---------- coercion helpers ---------- */

fn bigdecimal_to_decimal(bd: &BigDecimal) -> ODataBuildResult<Decimal> {
    // Robust conversion: preserve precision via string.
    let s = bd.normalized().to_string();
    Decimal::from_str_exact(&s)
        .or_else(|_| s.parse::<Decimal>())
        .map_err(|_| ODataBuildError::Other("invalid decimal"))
}

fn coerce(kind: FieldKind, v: &core::Value) -> ODataBuildResult<sea_orm::Value> {
    use core::Value as V;
    Ok(match (kind, v) {
        (FieldKind::String, V::String(s)) => sea_orm::Value::String(Some(Box::new(s.clone()))),

        (FieldKind::I64, V::Number(n)) => {
            let i = n.to_i64().ok_or(ODataBuildError::TypeMismatch {
                expected: FieldKind::I64,
                got: "number",
            })?;
            sea_orm::Value::BigInt(Some(i))
        }

        (FieldKind::F64, V::Number(n)) => {
            let f = n.to_f64().ok_or(ODataBuildError::TypeMismatch {
                expected: FieldKind::F64,
                got: "number",
            })?;
            sea_orm::Value::Double(Some(f))
        }

        // 🔧 Box the Decimal
        (FieldKind::Decimal, V::Number(n)) => {
            sea_orm::Value::Decimal(Some(Box::new(bigdecimal_to_decimal(n)?)))
        }

        (FieldKind::Bool, V::Bool(b)) => sea_orm::Value::Bool(Some(*b)),

        // 🔧 Box the Uuid
        (FieldKind::Uuid, V::Uuid(u)) => sea_orm::Value::Uuid(Some(Box::new(*u))),

        // 🔧 Box chrono types
        (FieldKind::DateTimeUtc, V::DateTime(dt)) => {
            sea_orm::Value::ChronoDateTimeUtc(Some(Box::new(*dt)))
        }
        (FieldKind::Date, V::Date(d)) => sea_orm::Value::ChronoDate(Some(Box::new(*d))),
        (FieldKind::Time, V::Time(t)) => sea_orm::Value::ChronoTime(Some(Box::new(*t))),

        (expected, V::Null) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "null",
            })
        }
        (expected, V::String(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "string",
            })
        }
        (expected, V::Number(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "number",
            })
        }
        (expected, V::Bool(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "bool",
            })
        }
        (expected, V::Uuid(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "uuid",
            })
        }
        (expected, V::DateTime(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "datetime",
            })
        }
        (expected, V::Date(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "date",
            })
        }
        (expected, V::Time(_)) => {
            return Err(ODataBuildError::TypeMismatch {
                expected,
                got: "time",
            })
        }
    })
}

fn coerce_many(kind: FieldKind, items: &[core::Expr]) -> ODataBuildResult<Vec<sea_orm::Value>> {
    items
        .iter()
        .map(|e| match e {
            core::Expr::Value(v) => coerce(kind, v),
            _ => Err(ODataBuildError::NonLiteralInList),
        })
        .collect()
}

/* ---------- LIKE helpers ---------- */

fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c => out.push(c),
        }
    }
    out
}
fn like_contains(s: &str) -> String {
    format!("%{}%", like_escape(s))
}
fn like_starts(s: &str) -> String {
    format!("{}%", like_escape(s))
}
fn like_ends(s: &str) -> String {
    format!("%{}", like_escape(s))
}

/* ---------- small guards ---------- */

#[inline]
fn ensure_string_field<E: EntityTrait>(f: &Field<E>, _field_name: &str) -> ODataBuildResult<()> {
    if f.kind != FieldKind::String {
        return Err(ODataBuildError::TypeMismatch {
            expected: FieldKind::String,
            got: "non-string field",
        });
    }
    Ok(())
}

/* ---------- Expr (AST) -> Condition ---------- */

pub fn expr_to_condition<E: EntityTrait>(
    expr: &core::Expr,
    fmap: &FieldMap<E>,
) -> ODataBuildResult<Condition>
where
    E::Column: ColumnTrait + Copy,
{
    use core::CompareOperator as Op;
    use core::Expr as X;

    Ok(match expr {
        X::And(a, b) => {
            let left = expr_to_condition::<E>(a, fmap)?;
            let right = expr_to_condition::<E>(b, fmap)?;
            Condition::all().add(left).add(right) // AND
        }
        X::Or(a, b) => {
            let left = expr_to_condition::<E>(a, fmap)?;
            let right = expr_to_condition::<E>(b, fmap)?;
            Condition::any().add(left).add(right) // OR
        }
        X::Not(x) => {
            let inner = expr_to_condition::<E>(x, fmap)?;
            // Use `all()` for consistency; semantically NOT ( ... )
            Condition::all().not().add(inner)
        }

        // Identifier op Value
        X::Compare(l, op, r) => {
            let (name, rhs) = match (&**l, &**r) {
                (X::Identifier(name), X::Value(v)) => (name, v),
                (X::Identifier(_), X::Identifier(_)) => {
                    return Err(ODataBuildError::Other(
                        "field-to-field comparison is not supported",
                    ))
                }
                _ => return Err(ODataBuildError::Other("unsupported comparison form")),
            };
            let f = fmap
                .get(name)
                .ok_or_else(|| ODataBuildError::UnknownField(name.clone()))?;
            let col = f.col;

            // null handling
            if matches!(rhs, core::Value::Null) {
                return Ok(match op {
                    Op::Eq => Condition::all().add(Expr::col(col).is_null()),
                    Op::Ne => Condition::all().add(Expr::col(col).is_not_null()),
                    _ => return Err(ODataBuildError::UnsupportedOp(*op)),
                });
            }

            let v = coerce(f.kind, rhs)?;
            let e = match op {
                Op::Eq => Expr::col(col).eq(v),
                Op::Ne => Expr::col(col).ne(v),
                Op::Gt => Expr::col(col).gt(v),
                Op::Ge => Expr::col(col).gte(v),
                Op::Lt => Expr::col(col).lt(v),
                Op::Le => Expr::col(col).lte(v),
            };
            Condition::all().add(e)
        }

        // Identifier IN (value, value, ...)
        X::In(l, list) => {
            let name = match &**l {
                X::Identifier(n) => n,
                _ => return Err(ODataBuildError::Other("left side of IN must be a field")),
            };
            let f = fmap
                .get(name)
                .ok_or_else(|| ODataBuildError::UnknownField(name.clone()))?;
            let col = f.col;
            let vals = coerce_many(f.kind, list)?;
            if vals.is_empty() {
                // IN () → always false
                Condition::all().add(Expr::cust("1=0"))
            } else {
                Condition::all().add(Expr::col(col).is_in(vals))
            }
        }

        // Supported functions: contains/startswith/endswith
        X::Function(fname, args) => {
            let n = fname.to_ascii_lowercase();
            match (n.as_str(), args.as_slice()) {
                ("contains", [X::Identifier(name), X::Value(core::Value::String(s))]) => {
                    let f = fmap
                        .get(name)
                        .ok_or_else(|| ODataBuildError::UnknownField(name.clone()))?;
                    ensure_string_field(f, name)?;
                    Condition::all().add(Expr::col(f.col).like(like_contains(s)))
                }
                ("startswith", [X::Identifier(name), X::Value(core::Value::String(s))]) => {
                    let f = fmap
                        .get(name)
                        .ok_or_else(|| ODataBuildError::UnknownField(name.clone()))?;
                    ensure_string_field(f, name)?;
                    Condition::all().add(Expr::col(f.col).like(like_starts(s)))
                }
                ("endswith", [X::Identifier(name), X::Value(core::Value::String(s))]) => {
                    let f = fmap
                        .get(name)
                        .ok_or_else(|| ODataBuildError::UnknownField(name.clone()))?;
                    ensure_string_field(f, name)?;
                    Condition::all().add(Expr::col(f.col).like(like_ends(s)))
                }
                _ => return Err(ODataBuildError::UnsupportedFn(fname.clone())),
            }
        }

        // Leaf forms are not valid WHERE by themselves
        X::Identifier(name) => return Err(ODataBuildError::BareIdentifier(name.clone())),
        X::Value(_) => return Err(ODataBuildError::BareLiteral),
    })
}

/// Apply an optional OData filter (via wrapper) to a plain SeaORM Select<E>.
///
/// This extension does NOT parse the filter string — it only consumes a parsed AST
/// (`odata_core::ast::Expr`) and translates it into a `sea_orm::Condition`.
pub trait ODataExt<E: EntityTrait>: Sized {
    fn apply_odata_filter(
        self,
        od_query: ODataQuery,
        fld_map: &FieldMap<E>,
    ) -> ODataBuildResult<Self>;
}

impl<E> ODataExt<E> for sea_orm::Select<E>
where
    E: EntityTrait,
    E::Column: ColumnTrait + Copy,
{
    fn apply_odata_filter(
        self,
        od_query: ODataQuery,
        fld_map: &FieldMap<E>,
    ) -> ODataBuildResult<Self> {
        match od_query.as_ast() {
            Some(ast) => {
                let cond = expr_to_condition::<E>(ast, fld_map)?;
                Ok(self.filter(cond))
            }
            None => Ok(self),
        }
    }
}
