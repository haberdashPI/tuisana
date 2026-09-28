//! A one-line text buffer with a caret, shared by every field that takes text.
//!
//! Beside `calendar.rs`, and for the same reason: a small interaction state
//! machine with no UI and no domain knowledge, owned by whoever is editing.
//! The filter panel and the task table's cell editor both hold one, so word
//! motion and line motion are written once rather than twice.

/// One of the forward cuts [`TextEdit::cut`] makes.
///
/// All three take text from the caret forward and hand it back to the caller,
/// which is what lets the app put it on the clipboard: a cut is a copy that
/// also deletes, so `ctrl-y` at the terminal, or any other paste, brings it
/// back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCut {
    /// The character under the caret.
    Char,
    /// From the caret to the end of the word ahead of it.
    Word,
    /// From the caret to the end of the line.
    ToEnd,
}

/// A one-line text buffer with a caret.
///
/// Positions are char indices, never byte offsets, because the caret is drawn
/// by reversing the character it sits on and a multi-byte character is one
/// caret stop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    caret: usize,
}

impl TextEdit {
    /// A buffer holding `text`, with the caret at its end.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let caret = text.chars().count();
        Self { text, caret }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret.min(self.len())
    }

    /// Replaces the text, clamping the caret into it.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.caret.min(self.len());
    }

    /// Replaces the text and puts the caret at its end.
    ///
    /// What an edit beginning wants: typing continues from where the value
    /// leaves off rather than from wherever the caret happened to be.
    pub fn set_text_at_end(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.len();
    }

    pub fn insert(&mut self, ch: char) {
        let mut chars = self.text.chars().collect::<Vec<_>>();
        let at = self.caret.min(chars.len());
        chars.insert(at, ch);
        self.text = chars.into_iter().collect();
        self.caret = at + 1;
    }

    pub fn delete_back(&mut self) {
        let mut chars = self.text.chars().collect::<Vec<_>>();
        let at = self.caret.min(chars.len());
        if at == 0 {
            return;
        }
        chars.remove(at - 1);
        self.text = chars.into_iter().collect();
        self.caret = at - 1;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn move_caret(&mut self, delta: i64) {
        let len = self.len() as i64;
        self.caret = (self.caret as i64 + delta).clamp(0, len) as usize;
    }

    /// Moves a word at a time: `-1` back, `1` forward. See [`word_boundary`].
    pub fn move_word(&mut self, delta: i64) {
        let chars = self.text.chars().collect::<Vec<_>>();
        let at = self.caret.min(chars.len());
        self.caret = word_boundary(&chars, at, delta);
    }

    /// Removes the text `cut` covers, answering what came out.
    ///
    /// The caret stays where it is: everything a cut takes is ahead of it, so
    /// the line closes up under an unmoved caret. An empty answer means there
    /// was nothing there to take, which is what stops `ctrl-d` at the end of a
    /// line from replacing the clipboard with nothing.
    pub fn cut(&mut self, cut: TextCut) -> String {
        let chars = self.text.chars().collect::<Vec<_>>();
        let from = self.caret.min(chars.len());
        let to = match cut {
            TextCut::Char => (from + 1).min(chars.len()),
            // The same boundary `alt-f` lands on, so what `alt-d` takes is
            // exactly what `alt-f` would have skipped over.
            TextCut::Word => word_boundary(&chars, from, 1),
            TextCut::ToEnd => chars.len(),
        };

        let taken = chars[from..to].iter().collect::<String>();
        if taken.is_empty() {
            return taken;
        }
        self.text = chars[..from].iter().chain(&chars[to..]).collect();
        self.caret = from;
        taken
    }

    pub fn jump_start(&mut self) {
        self.caret = 0;
    }

    pub fn jump_end(&mut self) {
        self.caret = self.len();
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }
}

/// Where a word motion from `from` lands: `-1` back, `1` forward.
///
/// Forward skips any non-word characters and then skips the run of word
/// characters it reaches, landing after it; back is the mirror image, looking
/// at the character *before* the position. Both clamp rather than wrap.
///
/// Shared by the motion and the cut so `alt-d` can only ever take what
/// `alt-f` would have moved over.
fn word_boundary(chars: &[char], from: usize, delta: i64) -> usize {
    let mut at = from.min(chars.len());

    if delta >= 0 {
        while at < chars.len() && !is_word(chars[at]) {
            at += 1;
        }
        while at < chars.len() && is_word(chars[at]) {
            at += 1;
        }
    } else {
        while at > 0 && !is_word(chars[at - 1]) {
            at -= 1;
        }
        while at > 0 && is_word(chars[at - 1]) {
            at -= 1;
        }
    }

    at
}

/// Whether a character is part of a word, for [`word_boundary`].
///
/// The regex `\w` class. Word motion stops on the `\b` boundaries this
/// implies, which is what makes `alt-f` in a hyphenated title stop at each
/// part rather than skipping the whole thing.
fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::{TextCut, TextEdit};

    #[test]
    fn word_motion_stops_on_word_boundaries() {
        // `\b`: a run of [A-Za-z0-9_]. Punctuation is skipped on the way and is
        // never a landing place, which is what makes `alt-f` in "(v2)" stop
        // after `v2` rather than after `)`.
        let cases = [
            ("Ship the release (v2)", 0, 1, 4),    // after "Ship"
            ("Ship the release (v2)", 4, 1, 8),    // after "the"
            ("Ship the release (v2)", 16, 1, 20),  // skips " (", stops after "v2"
            ("Ship the release (v2)", 21, 1, 21),  // clamps at the end
            ("Ship the release (v2)", 21, -1, 18), // skips ")", lands before "v2"
            ("Ship the release (v2)", 17, -1, 9),  // skips " ", lands before "release"
            ("Ship the release (v2)", 0, -1, 0),   // clamps at the start
        ];

        for (text, from, delta, want) in cases {
            let mut edit = TextEdit::new(text);
            edit.move_caret(from as i64 - edit.caret() as i64);
            assert_eq!(edit.caret(), from, "setup for {text:?} at {from}");
            edit.move_word(delta);
            assert_eq!(edit.caret(), want, "{text:?} from {from} by {delta}");
        }
    }

    #[test]
    fn caret_positions_are_characters_not_bytes() {
        let mut edit = TextEdit::new("日本語 テスト");

        assert_eq!(edit.caret(), 7, "seven characters, not seventeen bytes");
        edit.move_word(-1);
        assert_eq!(edit.caret(), 4, "before テスト");
        edit.move_word(-1);
        assert_eq!(edit.caret(), 0);
        edit.move_word(1);
        assert_eq!(edit.caret(), 3, "after 日本語");
    }

    #[test]
    fn editing_inserts_deletes_and_clears_at_the_caret() {
        let mut edit = TextEdit::new("abc");
        edit.move_caret(-1);
        edit.insert('X');

        assert_eq!(edit.text(), "abXc");
        assert_eq!(edit.caret(), 3);

        edit.delete_back();
        assert_eq!(edit.text(), "abc");

        edit.jump_start();
        edit.delete_back();
        assert_eq!(edit.text(), "abc", "backspace at the start does nothing");

        edit.jump_end();
        assert_eq!(edit.caret(), 3);

        edit.clear();
        assert_eq!(edit.text(), "");
        assert_eq!(edit.caret(), 0);
    }

    #[test]
    fn cuts_take_forward_from_the_caret_and_answer_what_they_took() {
        // (text, caret, cut, what came out, what is left)
        let cases = [
            ("Ship the release", 5, TextCut::Char, "t", "Ship he release"),
            ("Ship the release", 5, TextCut::Word, "the", "Ship  release"),
            ("Ship the release", 5, TextCut::ToEnd, "the release", "Ship "),
            // Mid-word: a cut takes the rest of the word it is standing in,
            // the way `alt-f` from there would have skipped only the rest.
            ("Ship the release", 6, TextCut::Word, "he", "Ship t release"),
            // The caret on a space takes the space with the word after it,
            // since that is where `alt-f` lands.
            ("Ship the release", 4, TextCut::Word, " the", "Ship release"),
            // Nothing ahead of the caret: every cut is a no-op, and answers
            // so, rather than emptying the clipboard.
            ("Ship", 4, TextCut::Char, "", "Ship"),
            ("Ship", 4, TextCut::Word, "", "Ship"),
            ("Ship", 4, TextCut::ToEnd, "", "Ship"),
        ];

        for (text, caret, cut, taken, left) in cases {
            let mut edit = TextEdit::new(text);
            edit.jump_start();
            edit.move_caret(caret as i64);

            assert_eq!(edit.cut(cut), taken, "{cut:?} from {caret} in {text:?}");
            assert_eq!(edit.text(), left, "{cut:?} from {caret} in {text:?}");
            assert_eq!(edit.caret(), caret, "the caret does not move");
        }
    }

    #[test]
    fn cuts_count_characters_not_bytes() {
        let mut edit = TextEdit::new("日本語 テスト");
        edit.jump_start();

        assert_eq!(edit.cut(TextCut::Word), "日本語");
        assert_eq!(edit.text(), " テスト");
        assert_eq!(edit.caret(), 0);

        assert_eq!(edit.cut(TextCut::Char), " ", "one character, not one byte");
        assert_eq!(edit.text(), "テスト");
    }

    #[test]
    fn replacing_the_text_clamps_the_caret_into_it() {
        let mut edit = TextEdit::new("a long value");
        edit.set_text("hi");

        assert_eq!(edit.caret(), 2);
    }
}
