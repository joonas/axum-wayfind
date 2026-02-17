// ==============================================================================
// StripPrefix Middleware
// ==============================================================================
//
// Strips a path prefix from incoming request URIs before forwarding to the
// inner service. Used by `Router::nest` and `Router::nest_service` to give
// nested handlers/services a view of the path relative to their mount point.
//
// Adapted from axum's internal `StripPrefix` implementation.

use http::{Request, Uri};
use std::{
    sync::Arc,
    task::{Context, Poll},
};
use tower_service::Service;

// ==============================================================================
// StripPrefixLayer
// ==============================================================================

/// A [`tower::Layer`] that wraps services with [`StripPrefix`].
#[allow(clippy::redundant_pub_crate)] // Explicit crate visibility on private-module item.
#[derive(Clone)]
pub(crate) struct StripPrefixLayer {
    prefix: Arc<str>,
}

impl StripPrefixLayer {
    #[allow(clippy::redundant_pub_crate)] // Explicit crate visibility on private-module item.
    pub(crate) fn new(prefix: &str) -> Self {
        Self {
            prefix: Arc::from(prefix),
        }
    }
}

impl<S> tower_layer::Layer<S> for StripPrefixLayer {
    type Service = StripPrefix<S>;

    fn layer(&self, inner: S) -> Self::Service {
        StripPrefix {
            inner,
            prefix: Arc::clone(&self.prefix),
        }
    }
}

// ==============================================================================
// StripPrefix Service
// ==============================================================================

/// Middleware that strips a path prefix from incoming request URIs.
#[allow(clippy::redundant_pub_crate)] // Explicit crate visibility on private-module item.
#[derive(Clone)]
pub(crate) struct StripPrefix<S> {
    inner: S,
    prefix: Arc<str>,
}

impl<S, B> Service<Request<B>> for StripPrefix<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        // If the prefix doesn't match (e.g. exact-prefix route with no
        // trailing path segments), forward the original URI unchanged.
        // This is intentional and matches axum's StripPrefix behavior.
        if let Some(new_uri) = strip_prefix(req.uri(), &self.prefix) {
            *req.uri_mut() = new_uri;
        }
        self.inner.call(req)
    }
}

// ==============================================================================
// strip_prefix -- URI manipulation
// ==============================================================================

/// Strip a path prefix from a URI, returning a new URI with the remaining path.
///
/// Walks the path and prefix segments in lockstep. For each matching pair,
/// accumulates how many bytes of the path are consumed by the prefix.
///
/// Example: prefix = "/api", path = "/api/users/42"
///          matched length = 4 ("/api"), remainder = "/users/42"
#[allow(clippy::option_if_let_else)] // Control flow is clearer with match here.
#[allow(clippy::expect_used)] // Invariant: stripping a valid prefix always yields a valid URI.
fn strip_prefix(uri: &Uri, prefix: &str) -> Option<Uri> {
    let path_and_query = uri.path_and_query()?;

    let mut matched_len = Some(0_usize);
    for item in zip_longest(segments(path_and_query.path()), segments(prefix)) {
        // Count the `/` separator between segments.
        *matched_len.as_mut()? += 1;

        match item {
            Item::Both(path_seg, prefix_seg) => {
                if is_capture(prefix_seg) || path_seg == prefix_seg {
                    *matched_len.as_mut()? += path_seg.len();
                } else if prefix_seg.is_empty() {
                    // Prefix ended with `/` -- e.g. prefix "/foo/" matched "/foo/bar".
                    break;
                } else {
                    matched_len = None;
                }
            }
            // Path has more segments than prefix -- the prefix matched.
            Item::First(_) => break,
            // Prefix has more segments than path -- no match.
            Item::Second(_) => {
                matched_len = None;
            }
        }
    }

    // The prefix always matches at a `/` boundary, so `split_at` won't panic.
    let after_prefix = uri.path().split_at(matched_len?).1;

    let new_path_and_query = match (after_prefix.starts_with('/'), path_and_query.query()) {
        (true, None) => after_prefix.parse().expect("valid path"),
        (true, Some(query)) => format!("{after_prefix}?{query}")
            .parse()
            .expect("valid path+query"),
        (false, None) => format!("/{after_prefix}").parse().expect("valid path"),
        (false, Some(query)) => format!("/{after_prefix}?{query}")
            .parse()
            .expect("valid path+query"),
    };

    // Build the new URI from parts without cloning the entire original URI.
    // For origin-form requests (the common case), scheme and authority are
    // both None, so this avoids any allocation beyond the path_and_query.
    let mut parts = http::uri::Parts::default();
    parts.scheme = uri.scheme().cloned();
    parts.authority = uri.authority().cloned();
    parts.path_and_query = Some(new_path_and_query);

    Some(Uri::from_parts(parts).expect("valid URI"))
}

// ==============================================================================
// Helpers
// ==============================================================================

/// Split a path into segments, skipping the leading empty segment from the
/// initial `/`.
fn segments(s: &str) -> impl Iterator<Item = &str> {
    s.split('/').skip(1)
}

/// Iterate two iterators in lockstep, continuing past the shorter one.
fn zip_longest<I, I2>(a: I, b: I2) -> impl Iterator<Item = Item<I::Item>>
where
    I: Iterator,
    I2: Iterator<Item = I::Item>,
{
    let a = a.map(Some).chain(std::iter::repeat_with(|| None));
    let b = b.map(Some).chain(std::iter::repeat_with(|| None));
    a.zip(b).map_while(|(a, b)| match (a, b) {
        (Some(a), Some(b)) => Some(Item::Both(a, b)),
        (Some(a), None) => Some(Item::First(a)),
        (None, Some(b)) => Some(Item::Second(b)),
        (None, None) => None,
    })
}

/// Check if a path segment is an axum-style capture (e.g. `{id}`), but not
/// a wildcard capture (`{*path}`) or escaped braces (`{{literal}}`).
fn is_capture(segment: &str) -> bool {
    segment.starts_with('{')
        && segment.ends_with('}')
        && !segment.starts_with("{{")
        && !segment.ends_with("}}")
        && !segment.starts_with("{*")
}

enum Item<T> {
    Both(T, T),
    First(T),
    Second(T),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)] // Tests panic on failure by design.

    use super::*;

    fn strip(uri: &str, prefix: &str) -> Option<String> {
        let uri: Uri = uri.parse().expect("valid URI");
        strip_prefix(&uri, prefix).map(|u| u.to_string())
    }

    #[test]
    fn static_prefix() {
        assert_eq!(strip("/api/users", "/api"), Some("/users".to_owned()));
    }

    #[test]
    fn static_prefix_with_trailing_slash() {
        assert_eq!(strip("/api/users", "/api/"), Some("/users".to_owned()));
    }

    #[test]
    fn exact_prefix_match() {
        assert_eq!(strip("/api", "/api"), Some("/".to_owned()));
    }

    #[test]
    fn no_match() {
        assert_eq!(strip("/other/users", "/api"), None);
    }

    #[test]
    fn preserves_query_string() {
        assert_eq!(
            strip("/api/users?page=1", "/api"),
            Some("/users?page=1".to_owned())
        );
    }

    #[test]
    fn dynamic_prefix_segment() {
        assert_eq!(strip("/v2/users", "/{version}"), Some("/users".to_owned()));
    }

    #[test]
    fn prefix_longer_than_path() {
        assert_eq!(strip("/api", "/api/v2"), None);
    }
}
