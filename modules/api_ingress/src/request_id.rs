use axum::http::{HeaderName, Request};
use axum::{body::Body, middleware::Next, response::Response};
use tower_http::request_id::{MakeRequestId, RequestId};
use tracing::field::Empty;

#[derive(Clone, Debug)]
pub struct XRequestId(pub String);

pub fn header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

#[derive(Clone, Default)]
pub struct MakeReqId;

impl MakeRequestId for MakeReqId {
    fn make_request_id<B>(&mut self, _req: &Request<B>) -> Option<RequestId> {
        // Generate a unique request ID using nanoid
        let id = nanoid::nanoid!();
        Some(RequestId::new(id.parse().ok()?))
    }
}

/// Middleware that stores request_id in Request.extensions and records it in the current span
pub async fn push_req_id_to_extensions(mut req: Request<Body>, next: Next) -> Response {
    let hdr = header();
    let rid = req
        .headers()
        .get(&hdr)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| "n/a".to_string());

    // Make it available to handlers
    req.extensions_mut().insert(XRequestId(rid.clone()));

    // Ensure the current span has the request_id field recorded
    tracing::Span::current().record("request_id", tracing::field::display(&rid));

    next.run(req).await
}

/// Create trace layer with proper typing  
#[allow(clippy::type_complexity)]
pub fn create_trace_layer() -> tower_http::trace::TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    impl Fn(&Request<Body>) -> tracing::Span + Clone,
> {
    use tower_http::trace::TraceLayer;

    TraceLayer::new_for_http().make_span_with(|req: &Request<Body>| {
        let hdr = header();
        let rid = req
            .headers()
            .get(&hdr)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("n/a");
        tracing::info_span!(
            "http_request",
            method = %req.method(),
            uri = %req.uri().path(),
            version = ?req.version(),
            module = "api_ingress",
            endpoint = %req.uri().path(),
            request_id = %rid,
            status = Empty,
            latency_ms = Empty
        )
    })
}
