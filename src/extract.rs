//! Extractors for `axum-wayfind`.
//!
//! Re-exports [`Path`] and [`MatchedPath`] which read from our own request
//! extensions rather than axum's internal types.

/// Matched-path extractor that records which route pattern was matched.
pub mod matched_path;
/// Path parameter extractor with percent-decoding and serde deserialization.
pub mod path;

pub use matched_path::MatchedPath;
pub use path::Path;
