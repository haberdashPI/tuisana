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

/// What a run of adjacent matched characters is worth.
///
/// Much larger than anything else here, because a run is the thing a reader
/// can actually see: `ship` matching `Ship` is a different event from the
/// same four letters scattered through `Shelve the pipeline`.
const CONTIGUOUS_BONUS: i32 = 16;
/// What matching the very first character is worth.
const START_BONUS: i32 = 24;
/// What matching the start of a word is worth.
const BOUNDARY_BONUS: i32 = 12;
/// Most a candidate can be penalised for its length.
///
/// Capped so a long title can still win on the strength of its match; without
/// a cap, length would quietly become the ranking.
const LENGTH_PENALTY_CAP: i32 = 10;

/// How well `needle` matches `haystack`, higher being a tighter match.
///
/// `None` when it does not match at all, which makes this a strictly more
/// informative [`fuzzy_match`]: the two agree on *whether* something matches
/// and this one also says how well. Case-insensitive, and a **subsequence**
/// match rather than a substring one, so `shp rc` finds
/// `Ship the release candidate`.
///
/// The scale is deliberately unspecified beyond "bigger is better". It exists
/// to order a candidate list, not to be shown to anyone.
pub fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Some(0);
    }

    let hay = haystack
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let mut score = 0i32;
    let mut from = 0usize;
    let mut previous: Option<usize> = None;

    for pin in needle.chars().flat_map(char::to_lowercase) {
        let found = from + hay.get(from..)?.iter().position(|ch| *ch == pin)?;
        if previous.is_some_and(|previous| previous + 1 == found) {
            score += CONTIGUOUS_BONUS;
        }
        if found == 0 {
            score += START_BONUS;
        } else if !hay[found - 1].is_alphanumeric() {
            score += BOUNDARY_BONUS;
        }
        previous = Some(found);
        from = found + 1;
    }

    // Among equally good matches the shorter candidate is the one meant:
    // `alex` means `Alex Chen`, not `Alexandra Pemberton-Clarke`.
    Some(score - (hay.len() as i32 / 4).min(LENGTH_PENALTY_CAP))
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_match, fuzzy_score};

    /// The two agree on whether something matches at all, which is what lets
    /// a caller move from one to the other without changing what it finds.
    #[test]
    fn a_score_exists_for_exactly_the_strings_fuzzy_match_accepts() {
        for (haystack, needle) in [
            ("hello world", "hlw"),
            ("hello", "hello"),
            ("hello", "xyz"),
            ("anything", ""),
            ("", "a"),
        ] {
            assert_eq!(
                fuzzy_score(haystack, needle).is_some(),
                fuzzy_match(haystack, needle),
                "{haystack:?} / {needle:?}"
            );
        }
    }

    #[test]
    fn a_subsequence_matches_where_a_substring_would_not() {
        assert!(fuzzy_score("Ship the release candidate", "shp rc").is_some());
        assert!(!"Ship the release candidate".contains("shp rc"));
    }

    #[test]
    fn the_tighter_of_two_matches_scores_higher() {
        let tight = fuzzy_score("Alex Chen", "alex").expect("matches");
        let loose = fuzzy_score("Alexandra Pemberton-Clarke", "alex").expect("matches");
        assert!(tight > loose, "{tight} should beat {loose}");

        let run = fuzzy_score("Ship the release candidate", "ship").expect("matches");
        let scattered = fuzzy_score("Shelve the pipeline", "ship").expect("matches");
        assert!(run > scattered, "{run} should beat {scattered}");
    }

    #[test]
    fn a_match_at_a_word_boundary_beats_one_buried_mid_word() {
        let boundary = fuzzy_score("Release candidate", "cand").expect("matches");
        let buried = fuzzy_score("Uncandid release", "cand").expect("matches");
        assert!(boundary > buried, "{boundary} should beat {buried}");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(fuzzy_score("Ship It", "SHIP").is_some());
        assert!(fuzzy_score("SHIP IT", "ship").is_some());
    }

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
