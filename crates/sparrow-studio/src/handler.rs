use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::extract::Path;

use crate::embed::Assets;

pub fn router<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::new()
        .route("/__studio", get(studio_redirect))
        .route("/__studio/", get(studio_index))
        .route("/__studio/{*path}", get(studio_asset))
}

async fn studio_redirect() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(header::LOCATION, "/__studio/")
        .body(Body::empty())
        .unwrap()
}

async fn studio_index() -> impl IntoResponse {
    serve_file("index.html")
}

async fn studio_asset(Path(path): Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    serve_file(path)
}

// All data is compiled into the binary via rust-embed; no disk I/O occurs here.
// If debug-embed is ever enabled (disk reads at runtime), these callers must become async.
fn serve_file(path: &str) -> Response<Body> {
    match Assets::get(path) {
        Some(content) => {
            let mime = if path.ends_with(".html") {
                "text/html; charset=utf-8".to_string()
            } else {
                mime_guess::from_path(path).first_or_octet_stream().to_string()
            };
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime);
            if path.starts_with("assets/") {
                builder = builder.header(
                    header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable",
                );
            }
            builder.body(Body::from(content.data.into_owned())).unwrap()
        }
        // SPA fallback: serve index.html for unrecognised paths
        None => match Assets::get("index.html") {
            Some(index) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(index.data.into_owned()))
                .unwrap(),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Studio not built"))
                .unwrap(),
        },
    }
}
