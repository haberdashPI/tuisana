//! The description pane's session state: what it is showing and how far down.
//!
//! Beside [`crate::app::gantt::GanttViewState`] and for the same reason: this
//! is a viewport over rows the table has already loaded, not a property of the
//! data. It owns the open/closed flag, the gid it is pointed at, the scroll
//! within that gid's text, and the descriptions fetched so far.
//!
//! Nothing here fetches anything. `App` owns the request, as it does for task
//! data, because `App` is where the client lives.

use std::collections::HashMap;

use crate::asana::TaskComment;

/// A description as it was when it was fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CachedDescription {
    /// The `modified_at` the *loaded record* carried when the fetch went out.
    ///
    /// **The comparison is the invalidation.** A lookup checks this against
    /// the record's current timestamp, so an edit landing on a task makes its
    /// cached text a miss without anything having to remember to clear it —
    /// which is the failure mode a separate invalidation hook would have.
    ///
    /// The record's value rather than the server's copy of it, which the
    /// description request deliberately does not ask for. Keying on the
    /// server's would let the two disagree — a task modified between the list
    /// read and this one — and a mismatch that no later read can resolve is a
    /// cache that misses on every frame, which is to say a fetch loop.
    modified_at: Option<String>,
    text: String,
    comments: Vec<TaskComment>,
}

/// What the pane has to say about the task under the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptionContent {
    /// No task row under the cursor — a header, a spacer, or an empty table.
    NoTask,
    /// A fetch is out for this task.
    Loading,
    /// The fetch failed, with the message it failed with.
    Failed(String),
    /// There is something to read. Either half may be empty, but not both.
    Body {
        /// The description, normalized. Empty when the task has none.
        notes: String,
        /// The comments, oldest first. Empty when nobody has commented.
        comments: Vec<TaskComment>,
    },
    /// Nothing written on this task at all: no description and no comments.
    Empty,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptionState {
    visible: bool,
    /// The gid the pane is pointed at, as of the last time it was aimed.
    target: Option<String>,
    /// The first wrapped line on screen.
    scroll: usize,
    cache: HashMap<String, CachedDescription>,
    /// The gid a fetch is out for and the timestamp it was aimed at.
    pending: Option<(String, Option<String>)>,
    /// The wrapped line count and body height of the last frame drawn.
    ///
    /// Written by the renderer, which is the only thing that knows how wide
    /// the pane ended up and therefore how many lines the text wrapped to —
    /// the division of labour `set_visible_columns` already follows. A scroll
    /// key reads it rather than guessing a page size.
    metrics: Option<(usize, usize)>,
    /// Why the last fetch failed, and which task it was for.
    ///
    /// Held here rather than pushed through `TaskState::edit_notice`: the
    /// notice box is for a write the user asked for and did not get, and a
    /// read failing is this pane's own content.
    error: Option<(String, String)>,
}

impl DescriptionState {
    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn toggle_visible(&mut self) {
        self.visible = !self.visible;
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Points the pane at a task, resetting the scroll if it is a new one.
    ///
    /// Carrying an offset from one description to the next would open the
    /// next one in the middle of itself.
    pub fn aim_at(&mut self, gid: Option<&str>) {
        if self.target.as_deref() == gid {
            return;
        }
        self.target = gid.map(str::to_string);
        self.scroll = 0;
    }

    /// Whether a fetch is out right now.
    pub fn is_fetching(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the task the pane is pointed at has nothing written on it.
    ///
    /// False while the text is still on its way: "nothing to read" and "not
    /// read yet" are different answers, and only one of them is final.
    pub fn is_empty(&self) -> bool {
        matches!(self.content(self.pending_modified_at()), DescriptionContent::Empty)
    }

    /// The timestamp the cached entry for the current target was aimed at.
    fn pending_modified_at(&self) -> Option<&str> {
        self.cache
            .get(self.target.as_deref()?)
            .and_then(|entry| entry.modified_at.as_deref())
    }

    /// Whether the task the pane is pointed at failed to load.
    pub fn failed(&self) -> bool {
        match (&self.target, &self.error) {
            (Some(target), Some((failed, _))) => target == failed,
            _ => false,
        }
    }

    /// Whether `gid`'s text still has to be fetched.
    ///
    /// False while a fetch for it is already out, and false once it has
    /// failed: a failed read is retried when the cursor comes back to the row
    /// or on a refresh, not on every frame.
    pub fn needs_fetch(&self, gid: &str, modified_at: Option<&str>) -> bool {
        if self.pending.as_ref().is_some_and(|(pending, _)| pending == gid) {
            return false;
        }
        if self.error.as_ref().is_some_and(|(failed, _)| failed == gid) {
            return false;
        }
        !self.is_cached(gid, modified_at)
    }

    fn is_cached(&self, gid: &str, modified_at: Option<&str>) -> bool {
        self.cache
            .get(gid)
            .is_some_and(|entry| entry.modified_at.as_deref() == modified_at)
    }

    /// Records that a fetch has gone out for `gid`, aimed at `modified_at`.
    ///
    /// The timestamp is captured here rather than when the reply lands,
    /// because that is the version of the task the text being fetched belongs
    /// to — the record may have moved on by the time it arrives, and if it has
    /// then the entry really is stale and should miss.
    pub fn begin_fetch(&mut self, gid: &str, modified_at: Option<String>) {
        self.pending = Some((gid.to_string(), modified_at));
        self.error = None;
    }

    /// Stores a fetched description and its comments, under the timestamp the
    /// request was aimed at.
    pub fn finish_fetch(&mut self, gid: &str, text: String, comments: Vec<TaskComment>) {
        let modified_at = match self.pending.take() {
            Some((pending, modified_at)) if pending == gid => modified_at,
            // A reply with no outstanding request behind it: keep it, but
            // under no timestamp, so the next look re-reads rather than
            // trusting a version nobody recorded.
            other => {
                self.pending = other;
                None
            }
        };
        self.cache.insert(
            gid.to_string(),
            CachedDescription {
                modified_at,
                text,
                comments,
            },
        );
    }

    /// Records a failed fetch against the task it was for.
    pub fn fail_fetch(&mut self, gid: &str, message: String) {
        if self.pending.as_ref().is_some_and(|(pending, _)| pending == gid) {
            self.pending = None;
        }
        self.error = Some((gid.to_string(), message));
    }

    /// Throws the fetched text away, so the next look refetches.
    ///
    /// Called from `refresh`, which is what makes a description edited in the
    /// browser readable without restarting the session.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.pending = None;
        self.error = None;
    }

    /// What to draw for the task the pane is pointed at.
    pub fn content(&self, modified_at: Option<&str>) -> DescriptionContent {
        let Some(gid) = self.target.as_deref() else {
            return DescriptionContent::NoTask;
        };
        if let Some((failed, message)) = &self.error {
            if failed == gid {
                return DescriptionContent::Failed(message.clone());
            }
        }
        match self.cache.get(gid) {
            Some(entry) if entry.modified_at.as_deref() == modified_at => {
                let notes = normalize(&entry.text);
                // Empty means *nothing to read*, not "no description": a task
                // with three comments and no description has plenty to say,
                // and reporting it as empty would hide all of it.
                if notes.is_empty() && entry.comments.is_empty() {
                    return DescriptionContent::Empty;
                }
                DescriptionContent::Body {
                    notes,
                    comments: entry.comments.clone(),
                }
            }
            // A cached entry whose timestamp has moved on is a miss, and a
            // miss while the pane is open means a fetch is on its way.
            _ => DescriptionContent::Loading,
        }
    }

    /// Records the geometry of the frame just drawn, and clamps the scroll to
    /// it.
    ///
    /// The clamp is why this is called on every frame and not only when
    /// something moves: widening the terminal rewraps the text to fewer
    /// lines, and an offset left past the end would draw an empty pane.
    pub fn set_metrics(&mut self, line_count: usize, body_height: usize) {
        self.metrics = Some((line_count, body_height));
        self.scroll = self.scroll.min(line_count.saturating_sub(body_height));
    }

    /// Moves the scroll by `pages` half-panes.
    ///
    /// A no-op until a frame has been drawn: the page size is half the pane's
    /// body, and nothing else knows how tall that is.
    pub fn scroll_half_pages(&mut self, pages: isize) {
        let Some((line_count, body_height)) = self.metrics else {
            return;
        };
        // `max(1)` so a two-line pane still moves.
        let step = (body_height / 2).max(1) as isize;
        self.scroll_by(pages.saturating_mul(step), line_count, body_height);
    }

    /// Moves the scroll by `delta` lines, clamped to the text's length.
    fn scroll_by(&mut self, delta: isize, line_count: usize, body_height: usize) {
        let max = line_count.saturating_sub(body_height);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }
}

/// Tidies a description into something a pane can hold.
///
/// Asana hands back whatever was pasted into it, and the two fixes here are
/// the ones that would otherwise land in the renderer: Windows line endings
/// would each draw a stray glyph, and a tab is a width this renderer does not
/// measure.
///
/// Blank lines are **not** normalized here, and nor are they preserved: how
/// paragraphs and lists are spaced is a layout question, and `ui::description`
/// answers it by dropping the source's blank lines and inserting its own. That
/// is what makes a description typed with one newline between paragraphs read
/// the same as one typed with two.
pub fn normalize(text: &str) -> String {
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ");

    normalized
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize, DescriptionContent, DescriptionState, TaskComment};

    fn loaded() -> DescriptionState {
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", Some("ts1".to_string()));
        state.finish_fetch("t1", "a\nb\nc\nd\ne".to_string(), Vec::new());
        state
    }

    fn comment(text: &str) -> TaskComment {
        TaskComment {
            author: Some("Alex Chen".to_string()),
            created_at: Some("2026-08-12T09:31:00.000Z".to_string()),
            text: text.to_string(),
        }
    }

    fn body(notes: &str) -> DescriptionContent {
        DescriptionContent::Body {
            notes: notes.to_string(),
            comments: Vec::new(),
        }
    }

    #[test]
    fn a_cached_description_is_a_hit_until_the_task_is_modified() {
        let state = loaded();

        assert!(!state.needs_fetch("t1", Some("ts1")));
        assert!(
            state.needs_fetch("t1", Some("ts2")),
            "the comparison is the invalidation"
        );
        assert!(state.needs_fetch("t2", Some("ts1")), "a different task");
    }

    #[test]
    fn a_task_with_no_timestamp_is_still_cached_rather_than_refetched() {
        // The loop this guards against: key the entry on a value the record
        // cannot produce and every frame is a miss, which is a thread and a
        // request per frame. Aiming the fetch at the record's own value —
        // `None` included — is what makes the hit reachable.
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", None);
        state.finish_fetch("t1", "text".to_string(), Vec::new());

        assert!(!state.needs_fetch("t1", None));
        assert_eq!(state.content(None), body("text"));
    }

    #[test]
    fn clearing_the_cache_makes_every_task_a_miss_again() {
        let mut state = loaded();
        state.clear_cache();

        assert!(state.needs_fetch("t1", Some("ts1")));
    }

    #[test]
    fn a_fetch_already_out_is_not_asked_for_twice() {
        let mut state = DescriptionState::default();
        state.begin_fetch("t1", Some("ts1".to_string()));

        assert!(!state.needs_fetch("t1", Some("ts1")));
        assert!(state.needs_fetch("t2", Some("ts1")));
    }

    #[test]
    fn a_failed_fetch_reports_itself_and_is_not_retried_on_the_next_frame() {
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", Some("ts1".to_string()));
        state.fail_fetch("t1", "404 not found".to_string());

        assert_eq!(
            state.content(Some("ts1")),
            DescriptionContent::Failed("404 not found".to_string())
        );
        assert!(!state.needs_fetch("t1", Some("ts1")));
        // Another task is unaffected: the failure is recorded against the one
        // it happened to.
        assert!(state.needs_fetch("t2", Some("ts1")));
    }

    #[test]
    fn moving_to_another_task_resets_the_scroll_but_staying_does_not() {
        let mut state = loaded();
        state.set_metrics(5, 1);
        state.scroll_by(3, 5, 1);
        assert_eq!(state.scroll(), 3);

        state.aim_at(Some("t1"));
        assert_eq!(state.scroll(), 3, "the same task keeps its place");

        state.aim_at(Some("t2"));
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn the_scroll_clamps_at_both_ends() {
        let mut state = loaded();

        state.scroll_by(-1, 5, 2);
        assert_eq!(state.scroll(), 0, "already at the top");

        state.scroll_by(99, 5, 2);
        assert_eq!(state.scroll(), 3, "the last line stays on screen");

        // A pane taller than the text cannot scroll at all.
        state.scroll_by(99, 5, 10);
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn a_half_page_is_half_the_pane_the_renderer_drew() {
        let mut state = loaded();
        state.set_metrics(40, 10);

        state.scroll_half_pages(1);
        assert_eq!(state.scroll(), 5);
        state.scroll_half_pages(1);
        assert_eq!(state.scroll(), 10);
        state.scroll_half_pages(-1);
        assert_eq!(state.scroll(), 5);

        // A one-line pane halves to zero, which would be a key that does
        // nothing.
        let mut tiny = loaded();
        tiny.set_metrics(40, 1);
        tiny.scroll_half_pages(1);
        assert_eq!(tiny.scroll(), 1);
    }

    #[test]
    fn scrolling_before_the_first_frame_does_nothing() {
        let mut state = loaded();
        state.scroll_half_pages(1);

        assert_eq!(state.scroll(), 0, "no geometry to halve yet");
    }

    #[test]
    fn a_wider_pane_pulls_the_scroll_back_into_the_text() {
        let mut state = loaded();
        state.set_metrics(20, 4);
        state.scroll_by(99, 20, 4);
        assert_eq!(state.scroll(), 16);

        state.set_metrics(6, 4);
        assert_eq!(state.scroll(), 2);
    }

    #[test]
    fn with_no_task_under_the_cursor_there_is_nothing_to_show() {
        let mut state = loaded();
        state.aim_at(None);

        assert_eq!(state.content(Some("ts1")), DescriptionContent::NoTask);
    }

    #[test]
    fn comments_alone_are_something_to_read() {
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", None);
        // No description at all, but somebody has commented: reporting this
        // as empty would hide the only thing written on the task.
        state.finish_fetch("t1", String::new(), vec![comment("Shipped today.")]);

        assert_eq!(
            state.content(None),
            DescriptionContent::Body {
                notes: String::new(),
                comments: vec![comment("Shipped today.")],
            }
        );
    }

    #[test]
    fn neither_a_description_nor_a_comment_is_empty() {
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", None);
        state.finish_fetch("t1", "  \n ".to_string(), Vec::new());

        assert_eq!(state.content(None), DescriptionContent::Empty);
    }

    #[test]
    fn an_empty_description_is_not_a_missing_one() {
        let mut state = DescriptionState::default();
        state.aim_at(Some("t1"));
        state.begin_fetch("t1", None);
        state.finish_fetch("t1", "   \n\n  ".to_string(), Vec::new());

        assert_eq!(state.content(None), DescriptionContent::Empty);
    }

    #[test]
    fn normalization_fixes_what_would_otherwise_reach_the_renderer() {
        assert_eq!(normalize("a\r\nb"), "a\nb");
        assert_eq!(normalize("a\rb"), "a\nb");
        assert_eq!(normalize("a\tb"), "a    b");
        assert_eq!(normalize("a   \nb"), "a\nb", "trailing space");
        assert_eq!(normalize("\n\na\n\n"), "a", "no blank lines at the ends");
        assert_eq!(normalize("   \n  "), "", "whitespace only is empty");
        // Blank lines in the middle are left exactly as they came: the layout
        // drops them and spaces the blocks itself.
        assert_eq!(normalize("a\n\n\nb"), "a\n\n\nb");
    }
}
