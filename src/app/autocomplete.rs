//! Completion over a closed set of names, for the fields that hold references.
//!
//! Beside `text_edit.rs` and `calendar.rs`: one buffer, a list of picked
//! items, a list of candidates, and no UI at all. Two callers drive it — the
//! task table's cell editor for `Assignee` and `Projects`, and the filter
//! panel's `list` match mode — which is why it is a module rather than more
//! state on either of them.
//!
//! Three things shape the design:
//!
//! - **Candidates are supplied, never fetched.** They arrive as
//!   `(handle, display)` pairs when the editor opens, so the state machine is
//!   testable without a backend and knows nothing about Asana.
//! - **Nothing is entered that is not a candidate.** `tab` walks the matches
//!   with the typed prefix intact rather than picking one, so a wrong first
//!   match costs one more keystroke instead of an undo; text that resolves to
//!   nothing is refused at commit rather than sent as a name Asana will not
//!   take.
//! - **The value is a list.** `Assignee` caps it at one and `Projects` does
//!   not, which is the only difference between the two.

use crate::app::text_edit::TextEdit;

/// What separates two items when the list is rendered as one string.
const ITEM_SEPARATOR: &str = ", ";

/// One thing that can be picked: what to show, and what to send.
///
/// The display name is the one thing Asana will not take, so the handle — a
/// user gid, a project gid, or the literal `me` — travels beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub handle: String,
    pub display: String,
}

impl Candidate {
    pub fn new(handle: impl Into<String>, display: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            display: display.into(),
        }
    }
}

/// Why a commit could not resolve the text still in the buffer.
///
/// The caller phrases the message: "no one called Alex" and "Alex is not a
/// project" are the same refusal about two different kinds of thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// The text matches no candidate at all.
    Unknown(String),
    /// It matches several, and picking one for the user would be a guess.
    Ambiguous(String),
}

/// A list of picked items, typed with completion over a closed set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutocompleteState {
    /// The items picked so far, in the order they will be shown.
    items: Vec<Candidate>,
    /// The text being typed, which sits between the items at `item_caret`.
    buffer: TextEdit,
    /// Where the buffer sits in the item list, as an index into `items`.
    item_caret: usize,
    /// Everything that could be picked.
    candidates: Vec<Candidate>,
    /// The prefix the current `tab` cycle is walking, and how far it has got.
    ///
    /// Kept apart from the buffer because `tab` *replaces* the buffer with a
    /// candidate: without the original prefix, a second `tab` would be
    /// completing the first match rather than walking past it.
    cycle: Option<(String, usize)>,
    /// How many items this field can hold. `Assignee` is one; `Projects` is
    /// unbounded.
    max_items: usize,
}

impl AutocompleteState {
    /// Opens an editor holding `items`, with the caret after the last of them.
    pub fn new(items: Vec<Candidate>, candidates: Vec<Candidate>, max_items: usize) -> Self {
        let item_caret = items.len();
        Self {
            items,
            buffer: TextEdit::default(),
            item_caret,
            candidates,
            cycle: None,
            max_items: max_items.max(1),
        }
    }

    /// The items picked so far.
    pub fn items(&self) -> &[Candidate] {
        &self.items
    }

    /// The text still being typed.
    pub fn buffer(&self) -> &str {
        self.buffer.text()
    }

    /// The buffer itself, for the caret motions every text field shares.
    pub fn buffer_mut(&mut self) -> &mut TextEdit {
        &mut self.buffer
    }

    /// Everything that could be picked, whether or not it has been.
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// The whole value as one string: the items, with the buffer spliced in
    /// where the caret is.
    ///
    /// The cell editor draws this through the same caret window every other
    /// field uses, so a long list scrolls under the caret rather than being
    /// truncated.
    pub fn text(&self) -> String {
        self.text_and_caret().0
    }

    /// Where the caret sits in [`AutocompleteState::text`], as a char index.
    pub fn caret(&self) -> usize {
        self.text_and_caret().1
    }

    fn text_and_caret(&self) -> (String, usize) {
        // A separator is owed to whatever comes next, rather than written
        // eagerly: the buffer is usually empty, and an empty slot must not
        // leave a `, ` hanging off the end of the value.
        let mut text = String::new();
        let mut separator_owed = false;

        for item in self.items.iter().take(self.item_caret) {
            if separator_owed {
                text.push_str(ITEM_SEPARATOR);
            }
            text.push_str(&item.display);
            separator_owed = true;
        }

        let trailing = self.items.len().saturating_sub(self.item_caret);
        if separator_owed && (!self.buffer.is_empty() || trailing > 0) {
            text.push_str(ITEM_SEPARATOR);
            separator_owed = false;
        }

        let caret = text.chars().count() + self.buffer.caret();
        text.push_str(self.buffer.text());
        separator_owed |= !self.buffer.is_empty();

        for item in self.items.iter().skip(self.item_caret) {
            if separator_owed {
                text.push_str(ITEM_SEPARATOR);
            }
            text.push_str(&item.display);
            separator_owed = true;
        }

        (text, caret)
    }

    /// The candidates the typed text currently matches, in offer order.
    ///
    /// Prefix matches first, so `me` leads the list for someone who typed
    /// `me` rather than hiding behind every name with those letters in it.
    /// Everything already picked is left out: offering it again would be
    /// offering a no-op.
    pub fn matches(&self) -> Vec<&Candidate> {
        self.matches_for(self.cycle.as_ref().map_or(self.buffer.text(), |(typed, _)| typed))
    }

    fn matches_for(&self, typed: &str) -> Vec<&Candidate> {
        let needle = typed.trim().to_ascii_lowercase();
        let mut matches = self
            .candidates
            .iter()
            .filter(|candidate| {
                !self
                    .items
                    .iter()
                    .any(|item| item.handle == candidate.handle)
            })
            .filter(|candidate| {
                needle.is_empty() || candidate.display.to_ascii_lowercase().contains(&needle)
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|candidate| {
            !candidate.display.to_ascii_lowercase().starts_with(&needle)
        });
        matches
    }

    /// Where the `tab` cycle currently sits in [`AutocompleteState::matches`].
    pub fn highlighted(&self) -> Option<usize> {
        self.cycle.as_ref().map(|(_, index)| *index)
    }

    /// Inserts a typed character, which starts the candidate list over.
    pub fn push_char(&mut self, ch: char) {
        self.buffer.insert(ch);
        self.cycle = None;
    }

    /// Deletes a character, or the item before the caret when there is none.
    ///
    /// One key for both because the buffer is where the caret is: with
    /// nothing typed, the thing before the caret *is* the previous item.
    pub fn delete_back(&mut self) {
        self.cycle = None;
        if !self.buffer.is_empty() {
            self.buffer.delete_back();
            return;
        }
        if self.item_caret == 0 {
            return;
        }
        self.item_caret -= 1;
        self.items.remove(self.item_caret);
    }

    /// Moves the caret, within the typed text first and between the items at
    /// its ends.
    pub fn move_caret(&mut self, delta: i64) {
        if delta < 0 {
            if self.buffer.caret() > 0 {
                self.buffer.move_caret(-1);
                return;
            }
            self.item_caret = self.item_caret.saturating_sub(1);
            return;
        }
        if self.buffer.caret() < self.buffer.text().chars().count() {
            self.buffer.move_caret(1);
            return;
        }
        self.item_caret = (self.item_caret + 1).min(self.items.len());
    }

    /// Empties the whole list, items and typed text alike.
    pub fn clear(&mut self) {
        self.items.clear();
        self.buffer.clear();
        self.item_caret = 0;
        self.cycle = None;
    }

    /// Completes the typed prefix to the next candidate, or the previous one.
    ///
    /// Answers whether anything was completed: a prefix that matches nothing
    /// is worth saying so rather than silently doing nothing.
    pub fn complete(&mut self, delta: i32) -> bool {
        let typed = match &self.cycle {
            Some((typed, _)) => typed.clone(),
            None => self.buffer.text().to_string(),
        };

        let displays = self
            .matches_for(&typed)
            .into_iter()
            .map(|candidate| candidate.display.clone())
            .collect::<Vec<_>>();
        if displays.is_empty() {
            return false;
        }

        let next = match &self.cycle {
            Some((_, index)) => (*index as i32 + delta).rem_euclid(displays.len() as i32) as usize,
            // A first `tab` lands on the first match going forward and the
            // last one going back, so `shift-tab` is a way in rather than a
            // way back to where you already were.
            None => match delta >= 0 {
                true => 0,
                false => displays.len() - 1,
            },
        };

        self.buffer.set_text_at_end(displays[next].clone());
        self.cycle = Some((typed, next));
        true
    }

    /// Resolves whatever is still typed and hands back the finished list.
    ///
    /// The buffer has to name a candidate: an exact display name, or a
    /// fragment that only one candidate matches. Anything else is refused,
    /// because a name Asana will not take is worse than an edit that did not
    /// happen.
    pub fn commit(mut self) -> Result<Vec<Candidate>, Unresolved> {
        let typed = self.buffer.text().trim().to_string();
        if typed.is_empty() {
            return Ok(self.items);
        }

        let exact = self
            .candidates
            .iter()
            .find(|candidate| candidate.display.eq_ignore_ascii_case(&typed))
            .cloned();
        let resolved = match exact {
            Some(candidate) => candidate,
            None => match self.matches_for(&typed).as_slice() {
                [] => return Err(Unresolved::Unknown(typed)),
                [only] => (*only).clone(),
                _ => return Err(Unresolved::Ambiguous(typed)),
            },
        };

        self.push_item(resolved);
        Ok(self.items)
    }

    /// Adds an item at the caret, dropping the oldest when the cap is reached.
    ///
    /// The newest item is always the one kept: on `Assignee`, typing a name
    /// over someone else's is how you reassign a task, and the name just
    /// typed is unambiguously the one meant.
    fn push_item(&mut self, item: Candidate) {
        if self.items.iter().any(|existing| existing.handle == item.handle) {
            self.buffer.clear();
            return;
        }

        let mut at = self.item_caret.min(self.items.len());
        self.items.insert(at, item);
        at += 1;
        while self.items.len() > self.max_items {
            let drop = match at > 1 {
                true => 0,
                false => self.items.len() - 1,
            };
            self.items.remove(drop);
            if drop < at {
                at -= 1;
            }
        }
        self.item_caret = at;
        self.buffer.clear();
        self.cycle = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{AutocompleteState, Candidate, Unresolved};

    fn people() -> Vec<Candidate> {
        vec![
            Candidate::new("me", "me"),
            Candidate::new("u1", "Alex Chen"),
            Candidate::new("u2", "Jo Park"),
            Candidate::new("u3", "Priya Raman"),
        ]
    }

    fn projects() -> AutocompleteState {
        AutocompleteState::new(
            vec![Candidate::new("p1", "Northwind")],
            vec![
                Candidate::new("p1", "Northwind"),
                Candidate::new("p2", "Backlog"),
                Candidate::new("p3", "Bench"),
            ],
            usize::MAX,
        )
    }

    fn type_text(state: &mut AutocompleteState, text: &str) {
        for ch in text.chars() {
            state.push_char(ch);
        }
    }

    #[test]
    fn tab_completes_the_prefix_and_repeating_it_walks_the_matches() {
        let mut state = projects();
        type_text(&mut state, "b");

        assert!(state.complete(1));
        assert_eq!(state.buffer(), "Backlog");
        assert!(state.complete(1));
        assert_eq!(state.buffer(), "Bench", "the prefix survives the first match");
        assert!(state.complete(1));
        assert_eq!(state.buffer(), "Backlog", "and the walk wraps");
        assert!(state.complete(-1));
        assert_eq!(state.buffer(), "Bench");
    }

    #[test]
    fn a_prefix_that_matches_nothing_completes_nothing_and_says_so() {
        let mut state = projects();
        type_text(&mut state, "zzz");

        assert!(!state.complete(1));
        assert_eq!(state.buffer(), "zzz", "the text is left as typed");
        assert_eq!(
            state.commit(),
            Err(Unresolved::Unknown("zzz".to_string())),
            "and it is refused rather than sent"
        );
    }

    #[test]
    fn an_item_already_picked_is_not_offered_again() {
        let state = projects();

        let offered = state
            .matches()
            .into_iter()
            .map(|candidate| candidate.display.as_str())
            .collect::<Vec<_>>();
        assert_eq!(offered, vec!["Backlog", "Bench"]);
    }

    #[test]
    fn a_fragment_only_one_candidate_matches_resolves_and_several_do_not() {
        let mut state = projects();
        type_text(&mut state, "ackl");
        assert_eq!(
            state.clone().commit().map(handles),
            Ok(vec!["p1".to_string(), "p2".to_string()])
        );

        let mut state = projects();
        type_text(&mut state, "b");
        assert_eq!(
            state.commit(),
            Err(Unresolved::Ambiguous("b".to_string())),
            "picking one of two would be a guess"
        );
    }

    #[test]
    fn backspace_deletes_a_character_and_then_the_item_before_the_caret() {
        let mut state = projects();
        type_text(&mut state, "ab");

        state.delete_back();
        assert_eq!(state.buffer(), "a");
        state.delete_back();
        assert_eq!(state.buffer(), "");
        state.delete_back();
        assert!(state.items().is_empty(), "then the item itself goes");
        state.delete_back();
        assert!(state.items().is_empty(), "and the start of the list holds");
    }

    #[test]
    fn the_caret_walks_between_the_items() {
        let mut state = AutocompleteState::new(
            vec![Candidate::new("p1", "Alpha"), Candidate::new("p2", "Beta")],
            Vec::new(),
            usize::MAX,
        );

        assert_eq!(state.text(), "Alpha, Beta");
        assert_eq!(state.caret(), "Alpha, Beta".chars().count());
        state.move_caret(-1);
        assert_eq!(state.text(), "Alpha, Beta", "the text is unchanged");
        assert_eq!(state.caret(), "Alpha, ".chars().count());
        state.delete_back();
        assert_eq!(
            handles(state.items().to_vec()),
            vec!["p2".to_string()],
            "backspace at the caret takes the item before it, not the last one"
        );
    }

    #[test]
    fn a_single_valued_field_replaces_what_it_holds() {
        let mut state = AutocompleteState::new(
            vec![Candidate::new("u1", "Alex Chen")],
            people(),
            1,
        );
        type_text(&mut state, "Jo Park");

        assert_eq!(state.commit().map(handles), Ok(vec!["u2".to_string()]));
    }

    #[test]
    fn clearing_empties_the_whole_list() {
        let mut state = projects();
        type_text(&mut state, "be");
        state.clear();

        assert_eq!(state.text(), "");
        assert_eq!(state.commit(), Ok(Vec::new()), "and commits as unset");
    }

    #[test]
    fn the_text_reads_as_the_list_it_is() {
        let mut state = projects();
        assert_eq!(state.text(), "Northwind");

        type_text(&mut state, "be");
        assert_eq!(state.text(), "Northwind, be");
        let items = state.commit().expect("resolves");
        assert_eq!(
            items.iter().map(|item| item.display.as_str()).collect::<Vec<_>>(),
            vec!["Northwind", "Bench"]
        );
    }

    fn handles(items: Vec<Candidate>) -> Vec<String> {
        items.into_iter().map(|item| item.handle).collect()
    }
}
