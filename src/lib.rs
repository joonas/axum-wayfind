//! # `axum-wayfind`
//!
//! An [Axum](https://docs.rs/axum) router backed by
//! [`wayfind`](https://docs.rs/wayfind) instead of
//! [`matchit`](https://docs.rs/matchit).
//!
//! Swap two imports and everything else stays the same:
//!
//! ```rust,ignore
//! // Before:
//! use axum::{Router, extract::Path};
//!
//! // After:
//! use axum_wayfind::{Router, extract::Path};
//! ```
//!
//! Handlers, method filters (`get`, `post`, …), middleware, `Json`, `State`,
//! and all other axum types work unchanged.

pub mod extract;
mod router;
mod syntax;

pub use router::{IntoMakeService, Router};
