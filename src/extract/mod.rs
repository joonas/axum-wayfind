//! Extractors for `axum-wayfind`.
//!
//! Re-exports [`Path`] and [`MatchedPath`] which read from our own request
//! extensions rather than axum's internal types.

pub mod matched_path;
pub mod path;

pub use matched_path::MatchedPath;
pub use path::Path;
