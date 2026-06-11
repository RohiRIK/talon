//! Embedded web console SPA (criterion 13) — `web/dist` compiled into the
//! binary via `rust-embed`, served at `/ui` with an SPA index fallback.
//! Static assets carry no secrets and are served unauthenticated; every data
//! call the SPA makes goes through the bearer-token-gated `/api/v1`.
//!
//! `web/dist` is a committed release artifact (rebuild: `bun run build` in
//! `web/`), so `cargo build --features web-ui` needs no bun/Node toolchain —
//! the same pattern as the prebuilt `hello.wasm` test fixture.

use axum::Router;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Assets;

/// Router serving the SPA. Nest at `/ui`.
pub fn router() -> Router {
    Router::new().fallback(serve_asset)
}

async fn serve_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(file) => respond(path, file),
        // SPA fallback: unknown non-asset paths get index.html so hash
        // routes survive a hard refresh; missing real assets stay 404.
        None if !path.starts_with("assets/") => match Assets::get("index.html") {
            Some(index) => respond("index.html", index),
            None => not_found(),
        },
        None => not_found(),
    }
}

fn respond(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    (
        [(header::CONTENT_TYPE, content_type(path))],
        file.data.into_owned(),
    )
        .into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// Minimal extension → MIME map for the handful of types Vite emits.
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn index_served_at_root_and_unknown_paths() {
        for uri in ["/", "/some/spa/route"] {
            let resp = router()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("req"),
                )
                .await
                .expect("response");
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
            let ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            assert!(ct.starts_with("text/html"), "{uri}: {ct}");
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("body");
            assert!(
                String::from_utf8_lossy(&body).contains("<div id=\"root\">"),
                "embedded index.html served offline"
            );
        }
    }

    #[tokio::test]
    async fn missing_asset_is_404_not_index() {
        let resp = router()
            .oneshot(
                Request::builder()
                    .uri("/assets/nope.js")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn content_types_cover_vite_outputs() {
        assert_eq!(content_type("assets/app.js"), "text/javascript");
        assert_eq!(content_type("assets/style.css"), "text/css");
        assert!(content_type("index.html").starts_with("text/html"));
        assert_eq!(content_type("weird.bin"), "application/octet-stream");
    }
}
