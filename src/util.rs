/*
    GC-Stats — API

    Shared utility functions, currently a helper to escape SQL `LIKE`
    wildcards in user-supplied search input.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

/// Escapes MySQL `LIKE` wildcards (`%`, `_`) and the escape character itself
/// so user input is matched literally instead of being interpreted as a pattern.
pub fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_like_escapes_wildcards() {
        assert_eq!(escape_like("100%_\\done"), "100\\%\\_\\\\done");
        assert_eq!(escape_like("plain"), "plain");
    }
}
