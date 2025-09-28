use crate::api::problem::{Problem, ProblemResponse};
use axum::http::StatusCode;
use odata_core::Error as ODataError;

/// Map OData errors to RFC 9457 Problem responses
///
/// This function handles the unified `Error` type for all OData-related errors.
/// It provides consistent Problem+JSON responses for all OData-related errors.
pub fn odata_error_to_problem(e: &ODataError, instance: &str) -> ProblemResponse {
    match e {
        // Pagination and cursor validation errors
        ODataError::OrderMismatch => {
            Problem::new(StatusCode::BAD_REQUEST, "Order Mismatch", "ORDER_MISMATCH")
                .with_code("ORDER_MISMATCH")
                .with_instance(instance)
                .into()
        }
        ODataError::FilterMismatch => Problem::new(
            StatusCode::BAD_REQUEST,
            "Filter Mismatch",
            "FILTER_MISMATCH",
        )
        .with_code("FILTER_MISMATCH")
        .with_instance(instance)
        .into(),
        ODataError::InvalidCursor => {
            Problem::new(StatusCode::BAD_REQUEST, "Invalid Cursor", "INVALID_CURSOR")
                .with_code("INVALID_CURSOR")
                .with_instance(instance)
                .into()
        }
        ODataError::InvalidLimit => Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Invalid Limit",
            "INVALID_LIMIT",
        )
        .with_code("INVALID_LIMIT")
        .with_instance(instance)
        .into(),
        ODataError::OrderWithCursor => Problem::new(
            StatusCode::BAD_REQUEST,
            "Order With Cursor",
            "Cannot specify both $orderby and cursor parameters",
        )
        .with_code("ORDER_WITH_CURSOR")
        .with_instance(instance)
        .into(),

        // Filter and OrderBy parsing errors
        ODataError::InvalidFilter(msg) => Problem::new(
            StatusCode::BAD_REQUEST,
            "Filter error",
            format!("invalid $filter: {}", msg),
        )
        .with_type("https://errors.example.com/ODATA_FILTER_INVALID")
        .with_code("ODATA_FILTER_INVALID")
        .with_instance(instance)
        .into(),
        ODataError::InvalidOrderByField(f) => Problem::new(
            StatusCode::BAD_REQUEST,
            "Unsupported OrderBy Field",
            format!("unsupported $orderby field: {}", f),
        )
        .with_code("UNSUPPORTED_ORDERBY_FIELD")
        .with_instance(instance)
        .into(),

        // Cursor parsing errors (all map to BAD_REQUEST with specific codes)
        ODataError::CursorInvalidBase64 => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor contains invalid base64url encoding",
        )
        .with_code("CURSOR_INVALID_BASE64")
        .with_instance(instance)
        .into(),
        ODataError::CursorInvalidJson => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor contains malformed JSON",
        )
        .with_code("CURSOR_INVALID_JSON")
        .with_instance(instance)
        .into(),
        ODataError::CursorInvalidVersion => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor version is not supported",
        )
        .with_code("CURSOR_INVALID_VERSION")
        .with_instance(instance)
        .into(),
        ODataError::CursorInvalidKeys => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor contains empty or invalid keys",
        )
        .with_code("CURSOR_INVALID_KEYS")
        .with_instance(instance)
        .into(),
        ODataError::CursorInvalidFields => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor contains empty or invalid fields",
        )
        .with_code("CURSOR_INVALID_FIELDS")
        .with_instance(instance)
        .into(),
        ODataError::CursorInvalidDirection => Problem::new(
            StatusCode::BAD_REQUEST,
            "Invalid Cursor",
            "Cursor contains invalid sort direction",
        )
        .with_code("CURSOR_INVALID_DIRECTION")
        .with_instance(instance)
        .into(),

        // Database and low-level errors
        ODataError::Db(_) => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Database Error",
            "An internal database error occurred",
        )
        .with_code("INTERNAL_DB")
        .with_instance(instance)
        .into(),
    }
}
