use crate::api::problem::{Problem, ProblemResponse};
use axum::http::StatusCode;
use odata_core::ODataPageError;

/// Map ODataPageError to RFC 9457 Problem once, so feature handlers don't do it
pub fn odata_page_error_to_problem(e: &ODataPageError, instance: &str) -> ProblemResponse {
    match e {
        ODataPageError::OrderMismatch => {
            Problem::new(StatusCode::BAD_REQUEST, "Order Mismatch", "ORDER_MISMATCH")
                .with_code("ORDER_MISMATCH")
                .with_instance(instance)
                .into()
        }
        ODataPageError::FilterMismatch => Problem::new(
            StatusCode::BAD_REQUEST,
            "Filter Mismatch",
            "FILTER_MISMATCH",
        )
        .with_code("FILTER_MISMATCH")
        .with_instance(instance)
        .into(),
        ODataPageError::InvalidCursor => {
            Problem::new(StatusCode::BAD_REQUEST, "Invalid Cursor", "INVALID_CURSOR")
                .with_code("INVALID_CURSOR")
                .with_instance(instance)
                .into()
        }
        ODataPageError::InvalidFilter(msg) => Problem::new(
            StatusCode::BAD_REQUEST,
            "Filter error",
            format!("invalid $filter: {}", msg),
        )
        .with_type("https://errors.example.com/ODATA_FILTER_INVALID")
        .with_code("ODATA_FILTER_INVALID")
        .with_instance(instance)
        .into(),
        ODataPageError::InvalidOrderByField(f) => Problem::new(
            StatusCode::BAD_REQUEST,
            "Unsupported OrderBy Field",
            format!("unsupported $orderby field: {}", f),
        )
        .with_code("UNSUPPORTED_ORDERBY_FIELD")
        .with_instance(instance)
        .into(),
        ODataPageError::InvalidLimit => Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Invalid Limit",
            "INVALID_LIMIT",
        )
        .with_code("INVALID_LIMIT")
        .with_instance(instance)
        .into(),
        ODataPageError::OrderWithCursor => Problem::new(
            StatusCode::BAD_REQUEST,
            "Order With Cursor",
            "Cannot specify both $orderby and cursor parameters",
        )
        .with_code("ORDER_WITH_CURSOR")
        .with_instance(instance)
        .into(),
        ODataPageError::Db(_) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Database Error",
            "An internal database error occurred",
        )
        .with_code("INTERNAL_DB")
        .with_instance(instance)
        .into(),
    }
}
