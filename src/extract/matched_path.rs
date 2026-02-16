// ==============================================================================
// MatchedPath Extractor
// ==============================================================================
//
// Our own `MatchedPath` since axum's constructor is `pub(crate)`. This
// extractor reads from an extension we insert during dispatch containing
// the original Axum-syntax template (e.g. "/users/{id}").

use axum_core::extract::FromRequestParts;
use axum_core::response::{IntoResponse, Response};
use http::{StatusCode, request::Parts};
use std::{convert::Infallible, fmt, sync::Arc};

/// Access the original route template that matched the current request.
///
/// The returned string is the route pattern as registered (e.g.
/// `"/users/{id}"`), **not** the actual request URI (e.g. `"/users/42"`).
///
/// ```rust,no_run
/// use axum_wayfind::{Router, extract::MatchedPath};
/// use axum::routing::get;
///
/// let app = Router::new().route(
///     "/users/{id}",
///     get(|path: MatchedPath| async move {
///         let path = path.as_str();
///         // `path` will be "/users/{id}"
///     })
/// );
/// # let _: Router = app;
/// ```
#[derive(Clone, Debug)]
pub struct MatchedPath(pub(crate) Arc<str>);

impl MatchedPath {
    /// Returns the original route template as a `str` (e.g. `"/users/{id}"`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S> FromRequestParts<S> for MatchedPath
where
    S: Send + Sync,
{
    type Rejection = MatchedPathRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or(MatchedPathRejection)
    }
}

/// Rejection for [`MatchedPath`] — returned when no matched path was
/// found in the request extensions.
#[derive(Debug)]
pub struct MatchedPathRejection;

impl IntoResponse for MatchedPathRejection {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Matched path is not available",
        )
            .into_response()
    }
}

impl fmt::Display for MatchedPathRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Matched path is not available")
    }
}

impl std::error::Error for MatchedPathRejection {}

// Also implement OptionalFromRequestParts so `Option<MatchedPath>` works.
impl<S> axum_core::extract::OptionalFromRequestParts<S> for MatchedPath
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().cloned())
    }
}
