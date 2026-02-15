// ==============================================================================
// Syntax Translation: Axum → wayfind
// ==============================================================================
//
// Axum uses `{param}` and `{*wildcard}` for path parameters, while wayfind
// uses `<param>` and `<*wildcard>`. We translate at route-insertion time so
// users write Axum-style paths and the wayfind engine receives its native
// syntax.

/// Translates an Axum-style path template to wayfind syntax.
///
/// - `{name}` → `<name>`
/// - `{*name}` → `<*name>`
///
/// Static segments and leading `/` are preserved as-is.
///
/// # Panics
///
/// Panics if a `{` is not closed by a matching `}`, or if a `}` appears
/// without a preceding `{`.
#[allow(clippy::redundant_pub_crate)] // Explicit crate visibility on private-module item.
#[allow(clippy::panic)] // Intentional: invalid path syntax is a programming error.
pub(crate) fn axum_to_wayfind(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            result.push('<');
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == '}' {
                    result.push('>');
                    closed = true;
                    break;
                }
                result.push(inner);
            }
            assert!(closed, "unclosed `{{` in path template: `{path}`");
        } else {
            assert!(ch != '}', "unmatched `}}` in path template: `{path}`");
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_path() {
        assert_eq!(axum_to_wayfind("/hello/world"), "/hello/world");
    }

    #[test]
    fn single_param() {
        assert_eq!(axum_to_wayfind("/users/{id}"), "/users/<id>");
    }

    #[test]
    fn multiple_params() {
        assert_eq!(
            axum_to_wayfind("/users/{user_id}/posts/{post_id}"),
            "/users/<user_id>/posts/<post_id>"
        );
    }

    #[test]
    fn wildcard() {
        assert_eq!(axum_to_wayfind("/files/{*path}"), "/files/<*path>");
    }

    #[test]
    fn root() {
        assert_eq!(axum_to_wayfind("/"), "/");
    }

    #[test]
    fn no_params() {
        assert_eq!(axum_to_wayfind("/static/page"), "/static/page");
    }

    #[test]
    #[should_panic(expected = "unclosed `{` in path template")]
    fn unclosed_brace_panics() {
        axum_to_wayfind("/users/{id");
    }

    #[test]
    #[should_panic(expected = "unmatched `}` in path template")]
    fn unmatched_close_brace_panics() {
        axum_to_wayfind("/users/id}");
    }
}
