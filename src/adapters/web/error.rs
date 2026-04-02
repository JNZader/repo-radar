use askama::Template;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use super::templates::ErrorTemplate;

/// Web layer error type with content-negotiation support.
#[derive(Debug, Clone)]
pub enum WebError {
    NotFound(String),
    InternalError(String),
    Conflict(String),
    BadRequest(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

impl WebError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
        }
    }

    fn variant_name(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::InternalError(_) => "internal_error",
            Self::Conflict(_) => "conflict",
            Self::BadRequest(_) => "bad_request",
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::NotFound(msg)
            | Self::InternalError(msg)
            | Self::Conflict(msg)
            | Self::BadRequest(msg) => msg,
        }
    }
}

/// Returns true if the Accept header prefers JSON over HTML.
fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|accept| accept.contains("application/json"))
        .unwrap_or(false)
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let status = self.status_code();

        if wants_json(&HeaderMap::default()) {
            // When called without request context, fall back to HTML.
            // In practice, the actual Accept header is checked via the
            // handler extracting headers and passing them explicitly.
        }

        // Default: return JSON for API clients, HTML for browsers.
        // Since IntoResponse doesn't have access to the request headers,
        // we return JSON by default — handlers can wrap with HTML if needed.
        let body = ErrorBody {
            error: self.variant_name().to_string(),
            message: self.message().to_string(),
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Content-negotiated error response that checks the Accept header.
pub fn into_negotiated_response(error: WebError, headers: &HeaderMap) -> Response {
    let status = error.status_code();

    if wants_json(headers) {
        let body = ErrorBody {
            error: error.variant_name().to_string(),
            message: error.message().to_string(),
        };
        (status, axum::Json(body)).into_response()
    } else {
        let tmpl = ErrorTemplate {
            status_code: status.as_u16(),
            title: status
                .canonical_reason()
                .unwrap_or("Error")
                .to_string(),
            message: error.message().to_string(),
        };
        let html = tmpl
            .render()
            .unwrap_or_else(|_| {
                format!(
                    "<html><body><h1>{} {}</h1><p>{}</p></body></html>",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("Error"),
                    error.message()
                )
            });
        (status, axum::response::Html(html)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn response_status(error: WebError) -> StatusCode {
        let response = error.into_response();
        response.status()
    }

    #[tokio::test]
    async fn not_found_returns_404() {
        let status = response_status(WebError::NotFound("page missing".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn internal_error_returns_500() {
        let status = response_status(WebError::InternalError("boom".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn conflict_returns_409() {
        let status = response_status(WebError::Conflict("already running".into())).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn bad_request_returns_400() {
        let status = response_status(WebError::BadRequest("invalid param".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn negotiated_response_returns_json_for_json_accept() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, "application/json".parse().unwrap());

        let response =
            into_negotiated_response(WebError::NotFound("gone".into()), &headers);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("\"error\":\"not_found\""));
    }

    #[tokio::test]
    async fn negotiated_response_returns_html_for_browser() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, "text/html".parse().unwrap());

        let response =
            into_negotiated_response(WebError::NotFound("gone".into()), &headers);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("<!DOCTYPE html>") || text.contains("<html>"));
        assert!(text.contains("gone"));
    }
}
