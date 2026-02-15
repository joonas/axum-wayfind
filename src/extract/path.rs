// ==============================================================================
// Path<T> Extractor
// ==============================================================================
//
// Our own `Path<T>` that reads from `WayfindUrlParams` (our extension type)
// instead of axum's internal `UrlParams`. The serde deserializer is ported
// from axum to work with our `PercentDecodedStr`.

pub(crate) mod de;

use axum_core::{
    extract::FromRequestParts,
    response::{IntoResponse, Response},
};
use http::{StatusCode, request::Parts};
use serde::de::DeserializeOwned;
use std::{fmt, ops::Deref, sync::Arc};

// ==============================================================================
// PercentDecodedStr
// ==============================================================================

/// A string that has been percent-decoded from a URL path parameter.
#[derive(Clone, Debug)]
pub struct PercentDecodedStr(Arc<str>);

impl PercentDecodedStr {
    /// Attempt to percent-decode the given string.
    ///
    /// # Errors
    ///
    /// Returns [`Utf8Error`](std::str::Utf8Error) if the decoded bytes are
    /// not valid UTF-8.
    pub fn new<S: AsRef<str>>(s: S) -> Result<Self, std::str::Utf8Error> {
        percent_encoding::percent_decode(s.as_ref().as_bytes())
            .decode_utf8()
            .map(|decoded| Self(decoded.as_ref().into()))
    }

    /// Returns the decoded string as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for PercentDecodedStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

/// Creates a `PercentDecodedStr` from an already-decoded string, without
/// performing any percent-decoding. Use [`PercentDecodedStr::new`] to
/// decode raw URL parameter values.
impl std::str::FromStr for PercentDecodedStr {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.into()))
    }
}

// ==============================================================================
// WayfindUrlParams — the extension type we insert during dispatch
// ==============================================================================

/// Extracted URL parameters from a wayfind match, stored as a request
/// extension. This is our equivalent of axum's internal `UrlParams`.
#[derive(Clone, Debug)]
pub enum WayfindUrlParams {
    /// Successfully decoded parameters.
    Params(Vec<(Arc<str>, PercentDecodedStr)>),
    /// A parameter contained bytes that are not valid UTF-8 after
    /// percent-decoding.
    InvalidUtf8InPathParam {
        /// The parameter name that contained invalid UTF-8.
        key: Arc<str>,
    },
}

impl WayfindUrlParams {
    /// Build `WayfindUrlParams` from a wayfind `Match`, percent-decoding each
    /// parameter value.
    #[must_use]
    pub fn from_match<T>(matched: &wayfind::Match<'_, '_, T>) -> Self {
        let mut params = Vec::with_capacity(matched.parameters.len());

        for (key, value) in &matched.parameters {
            let key: Arc<str> = Arc::from(*key);

            match PercentDecodedStr::new(*value) {
                Ok(decoded) => params.push((key, decoded)),
                Err(_) => return Self::InvalidUtf8InPathParam { key },
            }
        }

        Self::Params(params)
    }
}

// ==============================================================================
// Path<T>
// ==============================================================================

/// Extractor that deserializes path parameters from the URL.
///
/// Drop-in replacement for [`axum::extract::Path`]. Uses our own
/// `WayfindUrlParams` extension instead of axum's internal `UrlParams`.
///
/// ```rust,no_run
/// use axum_wayfind::{Router, extract::Path};
/// use axum::routing::get;
///
/// async fn handler(Path(id): Path<u32>) {
///     println!("user id: {id}");
/// }
///
/// let app = Router::new().route("/users/{id}", get(handler));
/// # let _: Router = app;
/// ```
#[derive(Debug)]
pub struct Path<T>(pub T);

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = PathRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract into a helper fn so it's compiled once regardless of T.
        fn get_params(parts: &Parts) -> Result<&[(Arc<str>, PercentDecodedStr)], PathRejection> {
            match parts.extensions.get::<WayfindUrlParams>() {
                Some(WayfindUrlParams::Params(params)) => Ok(params),
                Some(WayfindUrlParams::InvalidUtf8InPathParam { key }) => {
                    let err = PathDeserializationError {
                        kind: ErrorKind::InvalidUtf8InPathParam {
                            key: key.to_string(),
                        },
                    };
                    Err(PathRejection::FailedToDeserializePathParams(
                        FailedToDeserializePathParams(err),
                    ))
                }
                None => Err(PathRejection::MissingPathParams),
            }
        }

        match T::deserialize(de::PathDeserializer::new(get_params(parts)?)) {
            Ok(val) => Ok(Self(val)),
            Err(e) => Err(PathRejection::FailedToDeserializePathParams(
                FailedToDeserializePathParams(e),
            )),
        }
    }
}

// ==============================================================================
// Error types — ported from axum
// ==============================================================================

#[derive(Debug)]
pub(crate) struct PathDeserializationError {
    pub(crate) kind: ErrorKind,
}

impl PathDeserializationError {
    pub(crate) const fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn wrong_number_of_parameters() -> WrongNumberOfParameters<()> {
        WrongNumberOfParameters { got: () }
    }

    #[track_caller]
    pub(crate) const fn unsupported_type(name: &'static str) -> Self {
        Self::new(ErrorKind::UnsupportedType { name })
    }
}

pub(crate) struct WrongNumberOfParameters<G> {
    got: G,
}

impl<G> WrongNumberOfParameters<G> {
    #[allow(clippy::unused_self)]
    pub(crate) fn got<G2>(self, got: G2) -> WrongNumberOfParameters<G2> {
        WrongNumberOfParameters { got }
    }
}

impl WrongNumberOfParameters<usize> {
    pub(crate) const fn expected(self, expected: usize) -> PathDeserializationError {
        PathDeserializationError::new(ErrorKind::WrongNumberOfParameters {
            got: self.got,
            expected,
        })
    }
}

impl serde::de::Error for PathDeserializationError {
    #[inline]
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self {
            kind: ErrorKind::Message(msg.to_string()),
        }
    }
}

impl fmt::Display for PathDeserializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}

impl std::error::Error for PathDeserializationError {}

/// The kinds of errors that can happen when deserializing into a [`Path`].
#[must_use]
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The URI contained the wrong number of parameters.
    WrongNumberOfParameters {
        /// The number of actual parameters in the URI.
        got: usize,
        /// The number of expected parameters.
        expected: usize,
    },

    /// Failed to parse the value at a specific key into the expected type.
    ParseErrorAtKey {
        /// The key at which the value was located.
        key: String,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse the value at a specific index into the expected type.
    ParseErrorAtIndex {
        /// The index at which the value was located.
        index: usize,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse a value into the expected type.
    ParseError {
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// A parameter contained text that, once percent decoded, wasn't valid UTF-8.
    InvalidUtf8InPathParam {
        /// The key at which the invalid value was located.
        key: String,
    },

    /// Tried to deserialize into an unsupported type such as nested maps.
    UnsupportedType {
        /// The name of the unsupported type.
        name: &'static str,
    },

    /// Failed to deserialize the value with a custom deserialization error.
    DeserializeError {
        /// The key at which the invalid value was located.
        key: String,
        /// The value that failed to deserialize.
        value: String,
        /// The deserialization failure message.
        message: String,
    },

    /// Catch-all variant for errors that don't fit any other variant.
    Message(String),
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(error) => error.fmt(f),
            Self::InvalidUtf8InPathParam { key } => write!(f, "Invalid UTF-8 in `{key}`"),
            Self::WrongNumberOfParameters { got, expected } => {
                write!(
                    f,
                    "Wrong number of path arguments for `Path`. Expected {expected} but got {got}"
                )?;

                if *expected == 1 {
                    write!(
                        f,
                        ". Note that multiple parameters must be extracted with a tuple `Path<(_, _)>` or a struct `Path<YourParams>`"
                    )?;
                }

                Ok(())
            }
            Self::UnsupportedType { name } => write!(f, "Unsupported type `{name}`"),
            Self::ParseErrorAtKey {
                key,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse `{key}` with value `{value}` to a `{expected_type}`"
            ),
            Self::ParseError {
                value,
                expected_type,
            } => write!(f, "Cannot parse `{value}` to a `{expected_type}`"),
            Self::ParseErrorAtIndex {
                index,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse value at index {index} with value `{value}` to a `{expected_type}`"
            ),
            Self::DeserializeError {
                key,
                value,
                message,
            } => write!(f, "Cannot parse `{key}` with value `{value}`: {message}"),
        }
    }
}

// ==============================================================================
// Rejection types
// ==============================================================================

/// Rejection type for [`Path`] if the captured route params couldn't be
/// deserialized into the expected type.
#[derive(Debug)]
pub struct FailedToDeserializePathParams(PathDeserializationError);

impl FailedToDeserializePathParams {
    /// Get a reference to the underlying error kind.
    pub const fn kind(&self) -> &ErrorKind {
        &self.0.kind
    }

    /// Convert this error into the underlying error kind.
    pub fn into_kind(self) -> ErrorKind {
        self.0.kind
    }

    /// Get the response body text used for this rejection.
    #[must_use]
    pub fn body_text(&self) -> String {
        match self.0.kind {
            ErrorKind::Message(_)
            | ErrorKind::DeserializeError { .. }
            | ErrorKind::InvalidUtf8InPathParam { .. }
            | ErrorKind::ParseError { .. }
            | ErrorKind::ParseErrorAtIndex { .. }
            | ErrorKind::ParseErrorAtKey { .. } => format!("Invalid URL: {}", self.0.kind),
            ErrorKind::WrongNumberOfParameters { .. } | ErrorKind::UnsupportedType { .. } => {
                self.0.kind.to_string()
            }
        }
    }

    /// Get the status code used for this rejection.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self.0.kind {
            ErrorKind::Message(_)
            | ErrorKind::DeserializeError { .. }
            | ErrorKind::InvalidUtf8InPathParam { .. }
            | ErrorKind::ParseError { .. }
            | ErrorKind::ParseErrorAtIndex { .. }
            | ErrorKind::ParseErrorAtKey { .. } => StatusCode::BAD_REQUEST,
            ErrorKind::WrongNumberOfParameters { .. } | ErrorKind::UnsupportedType { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl IntoResponse for FailedToDeserializePathParams {
    fn into_response(self) -> Response {
        (self.status(), self.body_text()).into_response()
    }
}

impl fmt::Display for FailedToDeserializePathParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for FailedToDeserializePathParams {}

/// Rejection for the [`Path`] extractor.
#[derive(Debug)]
pub enum PathRejection {
    /// Failed to deserialize the path parameters.
    FailedToDeserializePathParams(FailedToDeserializePathParams),
    /// No path parameters were found in the request extensions.
    MissingPathParams,
}

impl IntoResponse for PathRejection {
    fn into_response(self) -> Response {
        match self {
            Self::FailedToDeserializePathParams(inner) => inner.into_response(),
            Self::MissingPathParams => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "No path parameters found",
            )
                .into_response(),
        }
    }
}

impl fmt::Display for PathRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FailedToDeserializePathParams(inner) => inner.fmt(f),
            Self::MissingPathParams => write!(f, "No path parameters found"),
        }
    }
}

impl std::error::Error for PathRejection {}
