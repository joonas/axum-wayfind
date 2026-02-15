# axum-wayfind

:warning: This is an experiment, please treat your usage of it as such. :warning:

An alternative [Axum](https://docs.rs/axum) router backed by [`wayfind`](https://docs.rs/wayfind)
instead of [`matchit`](https://docs.rs/matchit).

## Why?

Axum's built-in `Router` uses `matchit` for path matching. `matchit` is
fast and correct, but its route-conflict rules are strict — it rejects
route sets that `wayfind` handles without ambiguity. If you need more
flexible path parameter patterns, wildcard routing, or simply hit a
`matchit` conflict error with a route set that should be valid,
`axum-wayfind` lets you swap in `wayfind`'s router while keeping
everything else about Axum the same.

## What stays the same

Handlers, method filters (`get`, `post`, …), middleware, `Json`, `State`,
and all other Axum types work unchanged. The only things you replace are
`Router` and the extractors that read from matched path parameters
(`Path` and `MatchedPath`).

## Usage

Replace two imports:

```rust
// Before:
use axum::{Router, extract::Path};

// After:
use axum_wayfind::{Router, extract::Path};
```

Then build your app exactly as you would with Axum:

```rust
use axum::routing::get;
use axum_wayfind::{Router, extract::Path};

async fn get_user(Path(id): Path<u32>) -> String {
    format!("user {id}")
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(|| async { "Hello, world!" }))
        .route("/users/{id}", get(get_user));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
```

### Extractors

`axum_wayfind` provides its own `Path` and `MatchedPath` extractors.
These read from `axum-wayfind`'s own request extensions rather than
Axum's internal types, so you must import them from `axum_wayfind`:

```rust
use axum_wayfind::extract::{Path, MatchedPath};
```

All other Axum extractors (`Json`, `State`, `Query`, `Headers`, etc.)
are used directly from `axum` as usual.

### Supported Router APIs

- `route` / `route_service` — register handlers and services
- `merge` — combine routers
- `fallback` / `fallback_service` — custom 404 handling
- `layer` / `route_layer` — apply Tower middleware
- `with_state` — supply application state
- `into_make_service` — serve with `axum::serve`

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.
