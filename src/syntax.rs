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
pub fn axum_to_wayfind(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            result.push('<');
            for inner in chars.by_ref() {
                if inner == '}' {
                    result.push('>');
                    break;
                }
                result.push(inner);
            }
        } else {
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
}
