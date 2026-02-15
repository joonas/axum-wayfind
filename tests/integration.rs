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
