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
