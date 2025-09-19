use axum::extract::{FromRequestParts, Query};
use axum::http::{request::Parts, StatusCode};
use odata_core::ast;
use odata_params::filters as od;
use serde::Deserialize;

// Re-export ODataFilter from odata-core for convenience
pub use odata_core::ODataQuery;

#[derive(Deserialize, Default)]
pub struct FilterParam {
    #[serde(rename = "$filter")]
    pub filter: Option<String>,
}

const MAX_FILTER_LEN: usize = 8 * 1024;
const MAX_NODES: usize = 2000;

/// Extract and validate an OData `$filter` from request parts, returning a parser-agnostic AST.
/// - Parses with `odata_params`
/// - Enforces a length budget and an AST node budget
/// - Treats empty/whitespace `$filter` as "no filter"
pub async fn extract_odata_filter<S>(
    parts: &mut Parts,
    state: &S,
) -> Result<ODataQuery, (StatusCode, String)>
where
    S: Send + Sync,
{
    // Parse query; default if missing
    let Query(q) = Query::<FilterParam>::from_request_parts(parts, state)
        .await
        .unwrap_or_else(|_| Query(FilterParam::default()));

    // No $filter → no AST
    let Some(raw) = q.filter.as_ref() else {
        return Ok(ODataQuery::none());
    };

    // Treat empty/whitespace as "no filter"
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(ODataQuery::none());
    }

    if raw.len() > MAX_FILTER_LEN {
        return Err((StatusCode::BAD_REQUEST, "filter too long".into()));
    }

    // Parse into odata_params AST
    let ast_src = od::parse_str(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid $filter: {:?}", e)))?;

    // Complexity budget (node count)
    fn count_nodes(e: &od::Expr) -> usize {
        use od::Expr::*;
        match e {
            Value(_) | Identifier(_) => 1,
            Not(x) => 1 + count_nodes(x),
            And(a, b) | Or(a, b) | Compare(a, _, b) => 1 + count_nodes(a) + count_nodes(b),
            In(a, list) => 1 + count_nodes(a) + list.iter().map(count_nodes).sum::<usize>(),
            Function(_, args) => 1 + args.iter().map(count_nodes).sum::<usize>(),
        }
    }
    if count_nodes(&ast_src) > MAX_NODES {
        return Err((StatusCode::BAD_REQUEST, "filter too complex".into()));
    }

    // Convert to transport-agnostic core AST
    let core_expr: ast::Expr = ast_src.into();
    Ok(ODataQuery::some(core_expr))
}

use std::ops::Deref;

/// Simple Axum extractor for `$filter`, backed by `extract_odata_filter`.
/// Usage in handlers:
///   async fn list_users(OData(filter): OData, /* ... */) { /* use `filter` */ }
#[derive(Debug, Clone)]
pub struct OData(pub ODataQuery);

impl OData {
    #[inline]
    pub fn into_inner(self) -> ODataQuery {
        self.0
    }
}

impl Deref for OData {
    type Target = ODataQuery;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<ODataQuery> for OData {
    #[inline]
    fn as_ref(&self) -> &ODataQuery {
        &self.0
    }
}

impl From<OData> for ODataQuery {
    #[inline]
    fn from(x: OData) -> Self {
        x.0
    }
}

impl<S> FromRequestParts<S> for OData
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    #[allow(clippy::manual_async_fn)]
    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl core::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let filter = extract_odata_filter(parts, state).await?;
            Ok(OData(filter))
        }
    }
}
