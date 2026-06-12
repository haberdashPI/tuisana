//! Shared utility functions used by multiple app modules.

/// Returns true if every character of `needle` appears in `haystack` in order.
pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next();

    for candidate in haystack.chars() {
        if Some(candidate) == current {
            current = needle_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::fuzzy_match;

    #[test]
    fn empty_needle_always_matches() {
        assert!(fuzzy_match("anything", ""));
        assert!(fuzzy_match("", ""));
    }

    #[test]
    fn exact_match() {
        assert!(fuzzy_match("hello", "hello"));
    }

    #[test]
    fn subsequence_match() {
        assert!(fuzzy_match("hello world", "hlw"));
    }

    #[test]
    fn no_match() {
        assert!(!fuzzy_match("hello", "xyz"));
    }
}
