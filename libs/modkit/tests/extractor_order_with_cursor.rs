use axum::{
    body::to_bytes,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use modkit::api::odata::OData;
use tower::ServiceExt;

#[tokio::test]
async fn order_with_cursor_is_400() {
    // trivial route just to trigger extractor
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide both cursor and $orderby
    let req = Request::builder()
        .uri("/?cursor=eyJ2IjoxLCJrIjpbIjEiXS&$orderby=id%20desc")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Check body contains ORDER_WITH_CURSOR
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let s = String::from_utf8_lossy(&body);
    assert!(s.contains("ORDER_WITH_CURSOR") || s.contains("order_with_cursor"));
}

#[tokio::test]
async fn cursor_only_is_ok() {
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide only cursor
    let req = Request::builder()
        .uri("/?cursor=eyJ2IjoxLCJrIjpbIjEiXS")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Should be 400 due to invalid cursor format, but not ORDER_WITH_CURSOR
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let s = String::from_utf8_lossy(&body);
    assert!(!s.contains("ORDER_WITH_CURSOR"));
}

#[tokio::test]
async fn orderby_only_is_ok() {
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide only $orderby
    let req = Request::builder()
        .uri("/?$orderby=id%20desc")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
