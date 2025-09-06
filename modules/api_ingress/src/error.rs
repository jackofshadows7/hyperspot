use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("rate limited")]
    TooManyRequests,
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<&'a str>,
    #[cfg(feature = "debug-errors")]
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a str>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        use AppError::*;

        // Extract request_id from current span context if available
        // Note: We'll get the request_id from extensions in handlers
        let request_id = "unknown";

        #[allow(unused_variables)]
        let (status, code, safe_msg, details): (StatusCode, &str, &str, Option<&str>) = match &self
        {
            BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.as_str(), None),
            Unauthorized(m) => (StatusCode::UNAUTHORIZED, "unauthorized", m.as_str(), None),
            Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.as_str(), None),
            NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.as_str(), None),
            Conflict(m) => (StatusCode::CONFLICT, "conflict", m.as_str(), None),
            TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "rate limited",
                None,
            ),
            Internal(err) => {
                #[cfg(feature = "debug-errors")]
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "internal error",
                        Some(err.to_string().as_str()),
                    )
                }
                #[cfg(not(feature = "debug-errors"))]
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "internal error",
                        None,
                    )
                }
            }
        };

        match &self {
            Internal(err) => tracing::error!(
                request_id = %request_id,
                error = %err,
                status = status.as_u16(),
                "request failed"
            ),
            other => tracing::warn!(
                request_id = %request_id,
                error = %other,
                status = status.as_u16(),
                "request failed"
            ),
        }

        let body = ErrorBody {
            code,
            message: safe_msg,
            request_id: Some(request_id),
            #[cfg(feature = "debug-errors")]
            details,
        };
        (status, Json(body)).into_response()
    }
}
