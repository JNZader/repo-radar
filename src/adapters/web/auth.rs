use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Middleware that validates `Authorization: Bearer <token>` against the configured dashboard token.
///
/// If the token matches, the request proceeds normally. Otherwise, returns 401 Unauthorized.
pub async fn require_bearer_token(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // The expected token is stored in a request extension by the middleware layer.
    let expected = request
        .extensions()
        .get::<ExpectedToken>()
        .map(|t| t.0.as_str());

    let Some(expected) = expected else {
        // No token configured — this middleware should not have been applied.
        return next.run(request).await;
    };

    match extract_bearer_token(&headers) {
        Some(provided) if provided == expected => next.run(request).await,
        _ => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    }
}

/// Extracts the bearer token from the `Authorization` header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Wrapper type to store the expected token in request extensions.
#[derive(Clone)]
pub struct ExpectedToken(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::Request,
        middleware,
        routing::get,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn app_with_token(token: &str) -> Router {
        let expected = ExpectedToken(token.to_string());
        Router::new()
            .route("/", get(ok_handler))
            .layer(middleware::from_fn(require_bearer_token))
            .layer(axum::Extension(expected))
    }

    fn app_without_token() -> Router {
        Router::new().route("/", get(ok_handler))
    }

    #[tokio::test]
    async fn valid_token_passes() {
        let app = app_with_token("secret123");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Authorization", "Bearer secret123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn invalid_token_rejected() {
        let app = app_with_token("secret123");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_header_rejected() {
        let app = app_with_token("secret123");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_header_rejected() {
        let app = app_with_token("secret123");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Authorization", "Basic dXNlcjpwYXNz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn no_token_configured_open_access() {
        let app = app_without_token();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"ok");
    }
}
