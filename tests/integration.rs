#[cfg(test)]
#[allow(clippy::expect_used)] // Tests panic on failure by design.
mod tests {
    use axum::{
        Json,
        extract::State,
        routing::{get, post},
    };
    use axum_wayfind::{
        Router,
        extract::{MatchedPath, Path},
    };
    use http::StatusCode;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use tower::ServiceExt;

    // ==============================================================================
    // Test Helpers
    // ==============================================================================

    /// Send a request to the router and return the response.
    async fn send_request(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> axum::response::Response {
        let builder = http::Request::builder().method(method).uri(uri);
        let req = if let Some(body) = body {
            builder
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .expect("valid request")
        } else {
            builder
                .body(axum::body::Body::empty())
                .expect("valid request")
        };
        app.oneshot(req).await.expect("infallible")
    }

    async fn get_body(resp: axum::response::Response) -> String {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        String::from_utf8(body.to_vec()).expect("utf8")
    }

    // ==============================================================================
    // Basic Routing
    // ==============================================================================

    #[tokio::test]
    async fn static_route() {
        let app = Router::new().route("/hello", get(|| async { "world" }));

        let resp = send_request(app, "GET", "/hello", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "world");
    }

    #[tokio::test]
    async fn multiple_routes() {
        let app = Router::new()
            .route("/a", get(|| async { "route a" }))
            .route("/b", get(|| async { "route b" }));

        let resp = send_request(app.clone(), "GET", "/a", None).await;
        assert_eq!(get_body(resp).await, "route a");

        let resp = send_request(app, "GET", "/b", None).await;
        assert_eq!(get_body(resp).await, "route b");
    }

    #[tokio::test]
    async fn method_routing() {
        let app = Router::new().route("/item", get(|| async { "get" }).post(|| async { "post" }));

        let resp = send_request(app.clone(), "GET", "/item", None).await;
        assert_eq!(get_body(resp).await, "get");

        let resp = send_request(app, "POST", "/item", None).await;
        assert_eq!(get_body(resp).await, "post");
    }

    #[tokio::test]
    async fn merge_method_routers_on_same_path() {
        let app = Router::new()
            .route("/item", get(|| async { "get" }))
            .route("/item", post(|| async { "post" }));

        let resp = send_request(app.clone(), "GET", "/item", None).await;
        assert_eq!(get_body(resp).await, "get");

        let resp = send_request(app, "POST", "/item", None).await;
        assert_eq!(get_body(resp).await, "post");
    }

    // ==============================================================================
    // Path Extraction
    // ==============================================================================

    #[tokio::test]
    async fn single_path_param() {
        let app = Router::new().route(
            "/users/{id}",
            get(|Path(id): Path<u32>| async move { format!("user {id}") }),
        );

        let resp = send_request(app, "GET", "/users/42", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "user 42");
    }

    #[tokio::test]
    async fn multiple_path_params() {
        let app =
            Router::new().route(
                "/users/{user_id}/posts/{post_id}",
                get(|Path((uid, pid)): Path<(u32, u32)>| async move {
                    format!("user {uid} post {pid}")
                }),
            );

        let resp = send_request(app, "GET", "/users/1/posts/99", None).await;
        assert_eq!(get_body(resp).await, "user 1 post 99");
    }

    #[tokio::test]
    async fn struct_path_params() {
        #[derive(Deserialize)]
        struct Params {
            user_id: u32,
            team_id: String,
        }

        let app =
            Router::new().route(
                "/users/{user_id}/teams/{team_id}",
                get(|Path(p): Path<Params>| async move {
                    format!("user {} team {}", p.user_id, p.team_id)
                }),
            );

        let resp = send_request(app, "GET", "/users/5/teams/alpha", None).await;
        assert_eq!(get_body(resp).await, "user 5 team alpha");
    }

    #[tokio::test]
    async fn hashmap_path_params() {
        let app = Router::new().route(
            "/users/{user_id}/teams/{team_id}",
            get(|Path(params): Path<HashMap<String, String>>| async move {
                format!("user {} team {}", params["user_id"], params["team_id"])
            }),
        );

        let resp = send_request(app, "GET", "/users/5/teams/alpha", None).await;
        assert_eq!(get_body(resp).await, "user 5 team alpha");
    }

    #[tokio::test]
    async fn wildcard_path_param() {
        let app = Router::new().route(
            "/files/{*path}",
            get(|Path(path): Path<String>| async move { format!("file: {path}") }),
        );

        let resp = send_request(app, "GET", "/files/a/b/c.txt", None).await;
        assert_eq!(get_body(resp).await, "file: a/b/c.txt");
    }

    #[tokio::test]
    async fn percent_decoding() {
        let app = Router::new().route("/{key}", get(|Path(key): Path<String>| async move { key }));

        let resp = send_request(app, "GET", "/hello%20world", None).await;
        assert_eq!(get_body(resp).await, "hello world");
    }

    // ==============================================================================
    // MatchedPath
    // ==============================================================================

    #[tokio::test]
    async fn matched_path_extractor() {
        let app = Router::new().route(
            "/users/{id}",
            get(|path: MatchedPath| async move { path.as_str().to_owned() }),
        );

        let resp = send_request(app, "GET", "/users/42", None).await;
        assert_eq!(get_body(resp).await, "/users/{id}");
    }

    #[tokio::test]
    async fn matched_path_in_extensions() {
        let app = Router::new().route(
            "/users/{id}",
            get(|req: axum::extract::Request| async move {
                req.extensions()
                    .get::<MatchedPath>()
                    .expect("MatchedPath in extensions")
                    .as_str()
                    .to_owned()
            }),
        );

        let resp = send_request(app, "GET", "/users/42", None).await;
        assert_eq!(get_body(resp).await, "/users/{id}");
    }

    // ==============================================================================
    // Root Route
    // ==============================================================================

    #[tokio::test]
    async fn root_route() {
        let app = Router::new().route("/", get(|| async { "root" }));

        let resp = send_request(app, "GET", "/", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "root");
    }

    // ==============================================================================
    // Error Paths
    // ==============================================================================

    #[tokio::test]
    async fn path_param_type_mismatch_returns_400() {
        let app = Router::new().route(
            "/users/{id}",
            get(|Path(id): Path<u32>| async move { format!("user {id}") }),
        );

        let resp = send_request(app, "GET", "/users/abc", None).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn wrong_number_of_path_params() {
        // Route has two params but we extract as a single `u32`.
        // This is a programming error rather than bad user input, so
        // axum's Path extractor returns 500 for parameter count mismatches.
        let app = Router::new().route(
            "/users/{id}/posts/{post_id}",
            get(|Path(id): Path<u32>| async move { format!("user {id}") }),
        );

        let resp = send_request(app, "GET", "/users/1/posts/2", None).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn enum_path_param() {
        #[derive(Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Color {
            Red,
            Blue,
        }

        let app = Router::new().route(
            "/color/{color}",
            get(|Path(color): Path<Color>| async move {
                match color {
                    Color::Red => "red",
                    Color::Blue => "blue",
                }
            }),
        );

        let resp = send_request(app.clone(), "GET", "/color/red", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "red");

        let resp = send_request(app, "GET", "/color/green", None).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ==============================================================================
    // Fallback
    // ==============================================================================

    #[tokio::test]
    async fn default_404() {
        let app = Router::new().route("/exists", get(|| async { "ok" }));

        let resp = send_request(app, "GET", "/not-here", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn custom_fallback() {
        let app = Router::new()
            .route("/exists", get(|| async { "ok" }))
            .fallback(|| async { (StatusCode::IM_A_TEAPOT, "teapot") });

        let resp = send_request(app, "GET", "/not-here", None).await;
        assert_eq!(resp.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(get_body(resp).await, "teapot");
    }

    #[tokio::test]
    async fn fallback_service() {
        use tower::service_fn;

        let svc = service_fn(|_req: axum::extract::Request| async {
            Ok::<_, std::convert::Infallible>(axum::response::IntoResponse::into_response((
                StatusCode::SERVICE_UNAVAILABLE,
                "maintenance",
            )))
        });

        let app = Router::new()
            .route("/exists", get(|| async { "ok" }))
            .fallback_service(svc);

        let resp = send_request(app, "GET", "/not-here", None).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(get_body(resp).await, "maintenance");
    }

    // ==============================================================================
    // State
    // ==============================================================================

    #[tokio::test]
    async fn with_state() {
        #[derive(Clone)]
        struct AppState {
            greeting: String,
        }

        let app = Router::new()
            .route(
                "/greet/{name}",
                get(
                    |State(state): State<AppState>, Path(name): Path<String>| async move {
                        format!("{}, {name}!", state.greeting)
                    },
                ),
            )
            .with_state(AppState {
                greeting: "Hello".to_owned(),
            });

        let resp = send_request(app, "GET", "/greet/Alice", None).await;
        assert_eq!(get_body(resp).await, "Hello, Alice!");
    }

    // ==============================================================================
    // JSON
    // ==============================================================================

    #[tokio::test]
    async fn json_request_response() {
        #[derive(Deserialize, Serialize)]
        struct User {
            name: String,
            age: u32,
        }

        let app = Router::new().route(
            "/users",
            post(|Json(user): Json<User>| async move {
                Json(User {
                    name: user.name.to_uppercase(),
                    age: user.age,
                })
            }),
        );

        let body = serde_json::to_string(&serde_json::json!({
            "name": "alice",
            "age": 30
        }))
        .expect("serialize");

        let resp = send_request(app, "POST", "/users", Some(body)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = get_body(resp).await;
        let user: User = serde_json::from_str(&body).expect("deserialize");
        assert_eq!(user.name, "ALICE");
        assert_eq!(user.age, 30);
    }

    // ==============================================================================
    // Merge
    // ==============================================================================

    #[tokio::test]
    async fn merge_routers() {
        let users = Router::new().route("/users", get(|| async { "users" }));
        let posts = Router::new().route("/posts", get(|| async { "posts" }));

        let app = users.merge(posts);

        let resp = send_request(app.clone(), "GET", "/users", None).await;
        assert_eq!(get_body(resp).await, "users");

        let resp = send_request(app, "GET", "/posts", None).await;
        assert_eq!(get_body(resp).await, "posts");
    }

    // ==============================================================================
    // Layers
    // ==============================================================================

    #[tokio::test]
    async fn route_layer_applies_to_routes_only() {
        // `route_layer` should apply the layer to routes but not the fallback.
        // We use SetResponseHeader to add a custom header so we can observe the
        // layer's effect.
        use tower_http::set_header::SetResponseHeaderLayer;

        let app = Router::new()
            .route("/exists", get(|| async { "ok" }))
            .route_layer(SetResponseHeaderLayer::overriding(
                http::header::HeaderName::from_static("x-custom"),
                http::HeaderValue::from_static("layered"),
            ));

        // Route should have the header.
        let resp = send_request(app.clone(), "GET", "/exists", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-custom")
                .expect("header")
                .to_str()
                .expect("str"),
            "layered"
        );

        // Fallback (404) should NOT have the header.
        let resp = send_request(app, "GET", "/not-here", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(resp.headers().get("x-custom").is_none());
    }

    #[tokio::test]
    async fn layer_applies_to_routes_and_fallback() {
        use tower_http::set_header::SetResponseHeaderLayer;

        let app = Router::new()
            .route("/exists", get(|| async { "ok" }))
            .layer(SetResponseHeaderLayer::overriding(
                http::header::HeaderName::from_static("x-custom"),
                http::HeaderValue::from_static("layered"),
            ));

        // Route should have the header.
        let resp = send_request(app.clone(), "GET", "/exists", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("x-custom")
                .expect("header")
                .to_str()
                .expect("str"),
            "layered"
        );

        // Fallback should also have the header.
        let resp = send_request(app, "GET", "/not-here", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get("x-custom")
                .expect("header")
                .to_str()
                .expect("str"),
            "layered"
        );
    }

    // ==============================================================================
    // Merge (additional)
    // ==============================================================================

    #[tokio::test]
    async fn merge_routers_with_overlapping_methods() {
        // Two routers register different methods on the same path. After merge,
        // both methods should work.
        let router_get = Router::new().route("/item", get(|| async { "get" }));
        let router_post = Router::new().route("/item", post(|| async { "post" }));

        let app = router_get.merge(router_post);

        let resp = send_request(app.clone(), "GET", "/item", None).await;
        assert_eq!(get_body(resp).await, "get");

        let resp = send_request(app, "POST", "/item", None).await;
        assert_eq!(get_body(resp).await, "post");
    }

    // ==============================================================================
    // Nesting
    // ==============================================================================

    #[tokio::test]
    async fn nest_basic_dispatch() {
        let api = Router::new()
            .route("/users", get(|| async { "users" }))
            .route("/posts", get(|| async { "posts" }));

        let app = Router::new().nest("/api", api);

        let resp = send_request(app.clone(), "GET", "/api/users", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "users");

        let resp = send_request(app.clone(), "GET", "/api/posts", None).await;
        assert_eq!(get_body(resp).await, "posts");

        // Non-nested path should 404.
        let resp = send_request(app, "GET", "/users", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn nest_path_extraction() {
        let api = Router::new().route(
            "/users/{id}",
            get(|Path(id): Path<u32>| async move { format!("user {id}") }),
        );

        let app = Router::new().nest("/api", api);

        let resp = send_request(app, "GET", "/api/users/42", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(get_body(resp).await, "user 42");
    }

    #[tokio::test]
    async fn nest_matched_path() {
        let api = Router::new().route(
            "/users/{id}",
            get(|path: MatchedPath| async move { path.as_str().to_owned() }),
        );

        let app = Router::new().nest("/api", api);

        let resp = send_request(app, "GET", "/api/users/42", None).await;
        assert_eq!(get_body(resp).await, "/api/users/{id}");
    }

    #[tokio::test]
    async fn nest_strips_prefix_from_uri() {
        // Verify that handlers see the stripped URI path, not the full one.
        let api = Router::new().route(
            "/resource",
            get(|req: axum::extract::Request| async move { req.uri().path().to_owned() }),
        );

        let app = Router::new().nest("/api/v1", api);

        let resp = send_request(app, "GET", "/api/v1/resource", None).await;
        assert_eq!(get_body(resp).await, "/resource");
    }

    #[tokio::test]
    async fn nest_with_outer_routes() {
        let api = Router::new().route("/items", get(|| async { "items" }));

        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .nest("/api", api);

        let resp = send_request(app.clone(), "GET", "/health", None).await;
        assert_eq!(get_body(resp).await, "ok");

        let resp = send_request(app, "GET", "/api/items", None).await;
        assert_eq!(get_body(resp).await, "items");
    }

    #[tokio::test]
    async fn nest_with_state() {
        #[derive(Clone)]
        struct AppState {
            prefix: String,
        }

        let api =
            Router::new().route(
                "/greet",
                get(|State(state): State<AppState>| async move {
                    format!("hello from {}", state.prefix)
                }),
            );

        let app = Router::new().nest("/api", api).with_state(AppState {
            prefix: "api".to_owned(),
        });

        let resp = send_request(app, "GET", "/api/greet", None).await;
        assert_eq!(get_body(resp).await, "hello from api");
    }

    #[tokio::test]
    async fn nest_inner_fallback() {
        let api = Router::new()
            .route("/known", get(|| async { "known" }))
            .fallback(|| async { (StatusCode::IM_A_TEAPOT, "api fallback") });

        let app = Router::new().nest("/api", api);

        // Known inner route should work.
        let resp = send_request(app.clone(), "GET", "/api/known", None).await;
        assert_eq!(get_body(resp).await, "known");

        // Unknown inner path should use the inner fallback.
        let resp = send_request(app.clone(), "GET", "/api/unknown", None).await;
        assert_eq!(resp.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(get_body(resp).await, "api fallback");

        // Path outside the nest should use the outer (default 404) fallback.
        let resp = send_request(app, "GET", "/other", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn nest_service_basic() {
        use tower::service_fn;

        let svc = service_fn(|req: axum::extract::Request| async move {
            Ok::<_, std::convert::Infallible>(axum::response::IntoResponse::into_response(format!(
                "service saw: {}",
                req.uri().path()
            )))
        });

        let app = Router::new().nest_service("/api", svc);

        let resp = send_request(app.clone(), "GET", "/api/foo/bar", None).await;
        assert_eq!(get_body(resp).await, "service saw: /foo/bar");

        // Exact prefix should also work.
        let resp = send_request(app.clone(), "GET", "/api", None).await;
        assert_eq!(get_body(resp).await, "service saw: /");

        // Trailing slash should work.
        let resp = send_request(app, "GET", "/api/", None).await;
        assert_eq!(get_body(resp).await, "service saw: /");
    }

    #[tokio::test]
    async fn nest_service_with_router() {
        // Nest an axum-wayfind Router as a service under a prefix.
        let inner = Router::new()
            .route("/users", get(|| async { "users" }))
            .route(
                "/users/{id}",
                get(|Path(id): Path<u32>| async move { format!("user {id}") }),
            );

        let app = Router::new().nest_service("/api", inner);

        let resp = send_request(app.clone(), "GET", "/api/users", None).await;
        assert_eq!(get_body(resp).await, "users");

        let resp = send_request(app, "GET", "/api/users/7", None).await;
        assert_eq!(get_body(resp).await, "user 7");
    }

    #[test]
    #[should_panic(expected = "nesting at the root is not supported")]
    fn nest_at_root_panics() {
        drop(Router::<()>::new().nest("/", Router::new()));
    }

    #[test]
    #[should_panic(expected = "nesting at the root is not supported")]
    fn nest_empty_path_panics() {
        drop(Router::<()>::new().nest("", Router::new()));
    }

    #[test]
    #[should_panic(expected = "nest path must not contain wildcards")]
    fn nest_wildcard_path_panics() {
        drop(Router::<()>::new().nest("/{*catch_all}", Router::new()));
    }

    // ==============================================================================
    // route_service
    // ==============================================================================

    #[tokio::test]
    async fn route_service_any_method() {
        use tower::service_fn;

        let svc = service_fn(|_req: axum::extract::Request| async {
            Ok::<_, std::convert::Infallible>(axum::response::IntoResponse::into_response(
                "from service",
            ))
        });

        let app = Router::new().route_service("/svc", svc);

        let resp = send_request(app.clone(), "GET", "/svc", None).await;
        assert_eq!(get_body(resp).await, "from service");

        let resp = send_request(app, "POST", "/svc", None).await;
        assert_eq!(get_body(resp).await, "from service");
    }
}
