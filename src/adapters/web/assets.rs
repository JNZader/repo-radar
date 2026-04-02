use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
pub struct StaticAssets;

/// Serve embedded static files with the correct MIME type.
pub async fn serve_static(Path(path): Path<String>) -> impl IntoResponse {
    match StaticAssets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_assets_contains_app_css() {
        let file = StaticAssets::get("css/app.css");
        assert!(file.is_some(), "css/app.css should be embedded");
    }

    #[test]
    fn static_assets_contains_scan_js() {
        let file = StaticAssets::get("js/scan.js");
        assert!(file.is_some(), "js/scan.js should be embedded");
    }
}
