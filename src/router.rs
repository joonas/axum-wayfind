// ==============================================================================
// Router<S> — Axum-compatible router backed by wayfind
// ==============================================================================
//
// This module provides a `Router<S>` that mirrors axum's `Router` API but
// uses wayfind for path matching instead of matchit. It reuses axum's
// `MethodRouter` and handler/extractor ecosystem.
//
// We intentionally avoid using axum's `Route` type directly because its
// constructor (`Route::new`) and `layer` method are `pub(crate)`. Instead,
// all endpoints — including raw services — are wrapped via
// `axum::routing::any_service()` into `MethodRouter`.

use std::{
    collections::HashMap,
    convert::Infallible,
    fmt,
    future::{Future, ready},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::routing::MethodRouter;
use axum_core::{extract::Request, response::IntoResponse};
use http::StatusCode;
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    extract::matched_path::MatchedPath, extract::path::WayfindUrlParams, strip_prefix, syntax,
};

// ==============================================================================
// RouteId
// ==============================================================================

/// An opaque identifier for a registered route.
///
/// **Invariant**: `RouteId(n)` is always the index of the corresponding
/// `MethodRouter` in [`Router::routes`]. IDs are assigned sequentially
/// starting from 0, with no gaps — so `RouteId(n).0 < routes.len()` holds
/// for every live ID. Both [`Router::nest`] and [`Router::merge`] depend
/// on this identity when iterating `routes` by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RouteId(usize);

// ==============================================================================
// Fallback
// ==============================================================================

/// How to handle requests that don't match any registered route.
#[derive(Clone)]
enum Fallback<S> {
    /// Return a plain 404 Not Found.
    Default,
    /// Dispatch to a `MethodRouter` (from `.fallback()` or `.fallback_service()`).
    Handler(Box<MethodRouter<S>>),
}

impl<S> Fallback<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn with_state(self, state: S) -> Fallback<()> {
        match self {
            Self::Default => Fallback::Default,
            Self::Handler(mr) => Fallback::Handler(Box::new(mr.with_state(state))),
        }
    }
}

// ==============================================================================
// Nesting constants and helpers
// ==============================================================================

/// Private parameter name for the wildcard tail in nested routes.
/// Parameters with this prefix are filtered from user-visible extractors
/// in [`WayfindUrlParams::from_match`].
const NEST_TAIL_PARAM: &str = "__private_nest_tail";

/// Validate a nest path: must start with `/`, must not be empty or `"/"`,
/// and must not contain wildcard captures.
#[allow(clippy::panic)] // Intentional: invalid nest paths are programming errors.
fn validate_nest_path(path: &str) {
    assert!(
        !path.is_empty() && path != "/",
        "nesting at the root is not supported; use `merge` instead"
    );
    assert!(
        path.starts_with('/'),
        "nest path must start with `/`, got `{path}`"
    );
    assert!(
        !path
            .split('/')
            .any(|seg| seg.starts_with("{*") && seg.ends_with('}')),
        "nest path must not contain wildcards, got `{path}`"
    );
}

// ==============================================================================
// Router<S>
// ==============================================================================

/// An HTTP router backed by [`wayfind`] for path matching.
///
/// Drop-in replacement for [`axum::Router`] — swap the import, keep
/// everything else (handlers, extractors, middleware) unchanged.
///
/// ```rust,no_run
/// use axum_wayfind::Router;
/// use axum::routing::get;
///
/// let app = Router::new()
///     .route("/", get(|| async { "Hello, world!" }))
///     .route("/users/{id}", get(|| async { "user" }));
/// # let _: Router = app;
/// ```
#[derive(Clone)]
pub struct Router<S = ()> {
    /// wayfind path tree: maps translated templates to `RouteId`.
    wayfind: wayfind::Router<RouteId>,
    /// Route endpoints indexed by `RouteId`, all as `MethodRouter`.
    routes: Vec<MethodRouter<S>>,
    /// `RouteId` → original Axum-syntax template (for `MatchedPath`).
    route_id_to_path: HashMap<RouteId, Arc<str>>,
    /// Original Axum-syntax template → `RouteId` (for merge detection).
    path_to_route_id: HashMap<Arc<str>, RouteId>,
    /// What to do when no route matches.
    fallback: Fallback<S>,
}

impl<S> fmt::Debug for Router<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("routes", &self.routes.len())
            .finish_non_exhaustive()
    }
}

impl Default for Router<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Create a new, empty router.
    #[must_use]
    pub fn new() -> Self {
        Self {
            wayfind: wayfind::Router::new(),
            routes: Vec::new(),
            route_id_to_path: HashMap::new(),
            path_to_route_id: HashMap::new(),
            fallback: Fallback::Default,
        }
    }

    // =========================================================================
    // Route registration
    // =========================================================================

    /// Register a `MethodRouter` at the given path.
    ///
    /// If the same path was already registered with a `MethodRouter`, the two
    /// are merged (matching axum's behavior for composing HTTP methods on a
    /// single path).
    ///
    /// # Panics
    ///
    /// Panics if the path does not start with `/`, contains reserved
    /// `__private_`-prefixed parameter names, is otherwise invalid, or
    /// conflicts with an existing route.
    #[must_use]
    #[allow(clippy::panic)] // Intentional: builder panics on invalid routes, matching axum's API.
    pub fn route(self, path: &str, method_router: MethodRouter<S>) -> Self {
        // The `__private_` prefix is reserved for internal use by the nesting
        // infrastructure. Reject it in user-facing routes to prevent parameters
        // from being silently filtered out during path extraction.
        assert!(
            !path.contains("{__private_") && !path.contains("{*__private_"),
            "parameter names starting with `__private_` are reserved: `{path}`"
        );

        self.route_inner(path, method_router)
    }

    /// Internal route registration — no `__private_` name check.
    ///
    /// Used by `nest()` and `nest_service()` which legitimately register
    /// routes containing `__private_`-prefixed wildcard parameters.
    #[must_use]
    #[allow(clippy::panic)] // Intentional: builder panics on invalid routes, matching axum's API.
    fn route_inner(mut self, path: &str, method_router: MethodRouter<S>) -> Self {
        assert!(
            path.starts_with('/'),
            "path must start with `/`, got `{path}`"
        );

        let path_arc: Arc<str> = Arc::from(path);

        // If this path already exists, merge the method routers.
        if let Some(&existing_id) = self.path_to_route_id.get(&path_arc) {
            let existing = std::mem::take(&mut self.routes[existing_id.0]);
            self.routes[existing_id.0] = existing.merge(method_router);
            return self;
        }

        // New route — translate syntax and insert into wayfind.
        let route_id = RouteId(self.routes.len());
        let translated = syntax::axum_to_wayfind(path);

        self.wayfind
            .insert(&translated, route_id)
            .unwrap_or_else(|err| panic!("failed to insert route `{path}`: {err}"));

        self.routes.push(method_router);
        self.route_id_to_path
            .insert(route_id, Arc::clone(&path_arc));
        self.path_to_route_id.insert(path_arc, route_id);

        self
    }

    /// Register an arbitrary tower `Service` at the given path.
    ///
    /// The service handles all HTTP methods. Internally wraps via
    /// `axum::routing::any_service()`.
    ///
    /// # Panics
    ///
    /// Panics if the path does not start with `/`, is otherwise invalid, or
    /// conflicts with an existing route.
    #[must_use]
    pub fn route_service<T>(self, path: &str, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse + 'static,
        T::Future: Send + 'static,
    {
        self.route(path, axum::routing::any_service(service))
    }

    // =========================================================================
    // Nesting
    // =========================================================================

    /// Nest a router under a path prefix.
    ///
    /// All routes in `router` are re-registered with `path` prepended, and
    /// each handler sees the request URI with the prefix stripped — matching
    /// the behavior of [`axum::Router::nest`].
    ///
    /// If the nested router has a custom fallback, it is registered as a
    /// wildcard catch-all under the prefix. wayfind's priority rules ensure
    /// that explicit routes match before the catch-all.
    ///
    /// # Panics
    ///
    /// Panics if `path` is empty, `"/"`, doesn't start with `/`, or
    /// contains wildcards.
    #[must_use]
    #[allow(clippy::expect_used)] // Invariant: every RouteId has a corresponding path entry.
    #[allow(clippy::panic)] // Intentional: invalid nest paths are programming errors.
    pub fn nest(mut self, path: &str, router: Self) -> Self {
        validate_nest_path(path);

        let Self {
            routes,
            route_id_to_path,
            fallback,
            ..
        } = router;

        debug_assert_eq!(
            routes.len(),
            route_id_to_path.len(),
            "RouteId index invariant violated: routes and path map have different lengths"
        );

        let strip = strip_prefix::StripPrefixLayer::new(path);

        // Flatten: prepend the nest prefix to each inner route and
        // re-register it in the outer router. StripPrefix ensures handlers
        // see the URI relative to their original mount point.
        for (idx, method_router) in routes.into_iter().enumerate() {
            let id = RouteId(idx);
            let inner_path = route_id_to_path
                .get(&id)
                .expect("every route should have a path");
            let full_path = format!("{path}{inner_path}");
            let layered = method_router.layer(strip.clone());

            // When the inner route is "/", the full_path becomes e.g. "/api/".
            // Also register at the bare prefix ("/api") so requests without a
            // trailing slash still match.
            if inner_path.as_ref() == "/" {
                self = self.route(path, layered.clone());
            }

            self = self.route(&full_path, layered);
        }

        // If the inner router has a custom fallback, register it as a
        // wildcard catch-all under the prefix so requests that match the
        // prefix but not any specific inner route use the inner fallback.
        if let Fallback::Handler(fallback_mr) = fallback {
            // `strip` is still available here — the loop only cloned it.
            let layered = (*fallback_mr).layer(strip);

            // Catch-all for sub-paths under the prefix.
            // Uses `route_inner` because the wildcard contains a `__private_`
            // parameter that would be rejected by `route()`.
            let wildcard = format!("{path}/{{*{NEST_TAIL_PARAM}}}");
            self = self.route_inner(&wildcard, layered.clone());

            // Also handle the exact prefix for requests like GET /api
            // (only if no inner "/" route already occupies this path).
            if !self.path_to_route_id.contains_key(path) {
                self = self.route(path, layered);
            }
        }

        self
    }

    /// Nest an arbitrary tower [`Service`] under a path prefix.
    ///
    /// The service handles all HTTP methods for any path that starts with
    /// `path`. The request URI is stripped of the prefix before reaching
    /// the inner service.
    ///
    /// # Panics
    ///
    /// Panics if `path` is empty, `"/"`, doesn't start with `/`, or
    /// contains wildcards.
    #[must_use]
    #[allow(clippy::panic)] // Intentional: invalid nest paths are programming errors.
    pub fn nest_service<T>(self, path: &str, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse + 'static,
        T::Future: Send + 'static,
    {
        validate_nest_path(path);

        let stripped = strip_prefix::StripPrefixLayer::new(path).layer(service);
        let method_router = axum::routing::any_service(stripped);

        // Register three route variants to cover all sub-path forms:

        // 1. Wildcard: /prefix/{*tail} -- matches everything under the prefix.
        // Uses `route_inner` because the wildcard contains a `__private_`
        // parameter that would be rejected by `route()`.
        let wildcard = format!("{path}/{{*{NEST_TAIL_PARAM}}}");
        let mut this = self.route_inner(&wildcard, method_router.clone());

        // 2. Exact prefix: /prefix -- handles requests to the prefix itself.
        this = this.route(path, method_router.clone());

        // 3. Trailing slash: /prefix/ -- if the prefix doesn't already end
        //    with `/`, ensure /prefix/ also routes to the service.
        if !path.ends_with('/') {
            let with_slash = format!("{path}/");
            this = this.route(&with_slash, method_router);
        }

        this
    }

    // =========================================================================
    // Merge
    // =========================================================================

    /// Merge another router into this one.
    ///
    /// # Panics
    ///
    /// Panics if the two routers have conflicting routes.
    #[must_use]
    #[allow(clippy::expect_used)] // Invariant: every RouteId has a corresponding path entry.
    pub fn merge(mut self, other: Self) -> Self {
        let Self {
            routes,
            route_id_to_path,
            fallback,
            ..
        } = other;

        debug_assert_eq!(
            routes.len(),
            route_id_to_path.len(),
            "RouteId index invariant violated: routes and path map have different lengths"
        );

        for (old_id, method_router) in routes.into_iter().enumerate() {
            let old_id = RouteId(old_id);
            let path = route_id_to_path
                .get(&old_id)
                .expect("every route should have a path");

            self = self.route(path, method_router);
        }

        // Merge fallback: other's non-default fallback takes precedence.
        if let Fallback::Handler(h) = fallback {
            self.fallback = Fallback::Handler(h);
        }

        self
    }

    // =========================================================================
    // Fallback
    // =========================================================================

    /// Set a fallback handler for requests that don't match any route.
    #[must_use]
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static,
    {
        self.fallback = Fallback::Handler(Box::new(axum::routing::any(handler)));
        self
    }

    /// Set a fallback service for requests that don't match any route.
    #[must_use]
    pub fn fallback_service<T>(mut self, service: T) -> Self
    where
        T: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        T::Response: IntoResponse + 'static,
        T::Future: Send + 'static,
    {
        self.fallback = Fallback::Handler(Box::new(axum::routing::any_service(service)));
        self
    }

    // =========================================================================
    // Layers
    // =========================================================================

    /// Apply a [`tower::Layer`] to all routes currently registered in the
    /// router, but not to the fallback.
    #[must_use]
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request, Error = Infallible> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        for mr in &mut self.routes {
            let taken = std::mem::take(mr);
            *mr = taken.route_layer(layer.clone());
        }
        self
    }

    /// Apply a [`tower::Layer`] to all routes *and* the fallback.
    #[must_use]
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        // Apply to all route endpoints.
        for mr in &mut self.routes {
            let taken = std::mem::take(mr);
            *mr = taken.layer(layer.clone());
        }

        // Apply to the fallback too.
        match &mut self.fallback {
            Fallback::Default => {
                // Wrap the default 404 in a MethodRouter so the layer applies.
                let fallback_mr: MethodRouter<S> =
                    axum::routing::any(|| async { StatusCode::NOT_FOUND });
                self.fallback = Fallback::Handler(Box::new(fallback_mr.layer(layer)));
            }
            Fallback::Handler(mr) => {
                let taken = std::mem::take(mr);
                *mr = Box::new(taken.layer(layer));
            }
        }

        self
    }

    // =========================================================================
    // State
    // =========================================================================

    /// Supply the state, converting `Router<S>` into `Router<()>`.
    ///
    /// After calling this, the router implements `Service<Request>` and can
    /// be served directly.
    pub fn with_state(self, state: S) -> Router<()> {
        let routes = self
            .routes
            .into_iter()
            .map(|mr| mr.with_state(state.clone()))
            .collect();

        let fallback = self.fallback.with_state(state);

        Router {
            wayfind: self.wayfind,
            routes,
            route_id_to_path: self.route_id_to_path,
            path_to_route_id: self.path_to_route_id,
            fallback,
        }
    }

    // =========================================================================
    // IntoMakeService
    // =========================================================================

    /// Convert this router into a `MakeService`, suitable for use with
    /// `axum::serve`.
    #[must_use]
    pub const fn into_make_service(self) -> IntoMakeService<Self>
    where
        Self: Sized,
    {
        IntoMakeService { svc: self }
    }
}

// ==============================================================================
// Service<Request> for Router<()>
// ==============================================================================

// The Service impl only exists for `Router<()>` — i.e. after state has been
// provided (or when no state is needed). This matches axum's design.

impl Service<Request> for Router<()> {
    type Response = axum::response::Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[allow(clippy::expect_used)] // Invariant: every RouteId has a corresponding path entry.
    fn call(&mut self, mut req: Request) -> Self::Future {
        // Search the wayfind tree for a matching route.
        let path = req.uri().path().to_owned();

        match self.wayfind.search(&path) {
            Some(matched) => {
                let route_id = *matched.data;

                // Build the URL parameters from the wayfind match,
                // percent-decoding each value.
                let params = WayfindUrlParams::from_match(&matched);
                req.extensions_mut().insert(params);

                // Insert MatchedPath using the original Axum-syntax template.
                let template = self
                    .route_id_to_path
                    .get(&route_id)
                    .expect("every route should have a path");
                req.extensions_mut()
                    .insert(MatchedPath(Arc::clone(template)));

                let mut mr = self.routes[route_id.0].clone();
                Box::pin(async move { mr.call(req).await })
            }
            None => {
                // No route matched — invoke the fallback.
                match &self.fallback {
                    Fallback::Default => Box::pin(ready(Ok(StatusCode::NOT_FOUND.into_response()))),
                    Fallback::Handler(mr) => {
                        let mut mr = mr.clone();
                        Box::pin(async move { mr.call(req).await })
                    }
                }
            }
        }
    }
}

// ==============================================================================
// IntoMakeService
// ==============================================================================

/// A `MakeService` wrapper so `axum::serve(listener, router.into_make_service())`
/// works.
#[derive(Debug, Clone)]
pub struct IntoMakeService<Svc> {
    svc: Svc,
}

impl<Svc, T> Service<T> for IntoMakeService<Svc>
where
    Svc: Clone,
{
    type Response = Svc;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Svc, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _target: T) -> Self::Future {
        ready(Ok(self.svc.clone()))
    }
}
