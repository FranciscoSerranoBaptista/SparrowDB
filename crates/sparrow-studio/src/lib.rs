mod embed;
mod handler;

pub use handler::router as studio_router;

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn redirect_bare_studio() {
        let app = super::studio_router();
        let resp = app
            .oneshot(Request::builder().uri("/__studio").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(resp.headers()["location"], "/__studio/");
    }

    #[tokio::test]
    async fn index_returns_200() {
        let app = super::studio_router();
        let resp = app
            .oneshot(Request::builder().uri("/__studio/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn spa_fallback_unknown_path() {
        let app = super::studio_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/__studio/some/unknown/route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // SPA fallback serves index.html with 200
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
