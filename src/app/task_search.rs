//! The task table's `/` search: the query, how it matches, and what it found.
//!
//! A search is not a filter. The rows it finds are picked out where they
//! already are and the cursor is sent to one of them; nothing is hidden and
//! nothing is reordered, which is what makes `/` safe to type over a table
//! someone has spent several keystrokes assembling. The filter panel is the
//! other half of that pair, and it is the half that changes which rows exist.
//!
//! What is searched is the **title cell**, which is the one cell every row
//! kind has something to say in: a task's own text on a task row, and the
//! section's name on a section heading. A task's description is deliberately
//! not searched — it is fetched for the row under the cursor and no other, so
//! searching it would mean fetching the lot to answer one keystroke.

use regex::RegexBuilder;

use crate::{
    app::project_list::SearchMode,
    domain::{TaskRowKind, TaskTableModel, TITLE_COLUMN},
    util::fuzzy_match,
};

/// Live state for one search.
///
/// Held only while the keys are in one of the two search modes: every way out
/// of them drops it, so a highlight on screen always means `n` and `N` are one
/// keystroke from doing something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskSearchState {
    query: String,
    mode: SearchMode,
    /// The task the cursor sat on when `/` was pressed, by gid.
    ///
    /// A gid rather than a row index: the table is rebuilt under an open
    /// search every time a streaming load delivers a project, and `esc` has to
    /// put the cursor back on the row the reader left rather than on whatever
    /// has since slid into its place.
    origin: Option<String>,
    /// Rows of the current table whose title cell matches, ascending.
    ///
    /// Indices rather than gids, because a section heading has no gid — and
    /// they are safe as indices precisely because nothing stores them: every
    /// rebuild of the table recomputes this list before anything reads it.
    matches: Vec<usize>,
    /// The rows `n` and `N` visit, ascending and without duplicates.
    ///
    /// Not the same list as `matches`: the cursor only ever sits on a task
    /// row, so a matched section heading sends it to the first task *in that
    /// section* instead. An empty section is still highlighted and simply has
    /// nowhere to send it.
    targets: Vec<usize>,
    /// What went wrong compiling the pattern, in regex mode.
    error: Option<String>,
}

impl TaskSearchState {
    /// Opens a search from a cursor sitting on `origin`.
    pub(crate) fn new(origin: Option<String>) -> Self {
        Self {
            query: String::new(),
            // Fuzzy, unlike the project list's own search: a task title is a
            // sentence rather than a name, so the letters someone remembers
            // of it are rarely adjacent.
            mode: SearchMode::Fuzzy,
            origin,
            matches: Vec::new(),
            targets: Vec::new(),
            error: None,
        }
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn mode(&self) -> SearchMode {
        self.mode
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub(crate) fn set_mode(&mut self, mode: SearchMode) {
        self.mode = mode;
    }

    pub(crate) fn push_char(&mut self, ch: char) {
        self.query.push(ch);
    }

    pub(crate) fn pop_char(&mut self) {
        self.query.pop();
    }

    pub(crate) fn clear_query(&mut self) {
        self.query.clear();
    }

    /// How many rows `n` and `N` have to visit.
    pub(crate) fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// Whether this row is one of the ones found, for the renderer.
    pub(crate) fn matched_row(&self, index: usize) -> bool {
        self.matches.binary_search(&index).is_ok()
    }

    /// The 1-based position of `row` among the rows `n` walks, if it is one.
    ///
    /// `None` for a cursor that has been moved off the matches with `j` or
    /// `k`, which is a real state and reads as a count without a position.
    pub(crate) fn position_of(&self, row: usize) -> Option<usize> {
        self.targets
            .binary_search(&row)
            .ok()
            .map(|position| position + 1)
    }

    /// The row the cursor should be sent to when the query changes.
    ///
    /// The first match at or after where the search started, wrapping to the
    /// top when there is nothing below. Measured from the origin on every
    /// keystroke rather than from the cursor, so typing one more character
    /// never walks the cursor down the table.
    pub(crate) fn first_target(&self, origin_row: Option<usize>) -> Option<usize> {
        let origin_row = origin_row.unwrap_or(0);
        self.targets
            .iter()
            .copied()
            .find(|row| *row >= origin_row)
            .or_else(|| self.targets.first().copied())
    }

    /// The next row to visit from `row`, wrapping at the end.
    pub(crate) fn step(&self, row: Option<usize>, delta: i32) -> Option<usize> {
        let Some(row) = row else {
            return self.targets.first().copied();
        };
        match delta >= 0 {
            true => self
                .targets
                .iter()
                .copied()
                .find(|target| *target > row)
                .or_else(|| self.targets.first().copied()),
            false => self
                .targets
                .iter()
                .copied()
                .rev()
                .find(|target| *target < row)
                .or_else(|| self.targets.last().copied()),
        }
    }

    /// Recomputes what matches, against the table as it now stands.
    ///
    /// Called on every change to the query, the match mode, and the table
    /// itself. The pattern is compiled once for the whole pass, like the
    /// filter panel's regex rows, rather than once per row.
    pub(crate) fn refresh(&mut self, table: &TaskTableModel) {
        self.error = None;
        self.matches.clear();
        self.targets.clear();

        // An empty query finds nothing rather than everything. A filter row
        // left blank is "no opinion"; a search with nothing typed into it has
        // not been asked yet, and highlighting every row would answer it.
        if self.query.is_empty() {
            return;
        }

        let regex = match self.mode {
            SearchMode::Regex => match RegexBuilder::new(&self.query)
                .case_insensitive(true)
                .build()
            {
                Ok(regex) => Some(regex),
                Err(error) => {
                    self.error = Some(error.to_string());
                    return;
                }
            },
            _ => None,
        };
        let needle = self.query.to_ascii_lowercase();

        for (index, row) in table.rows.iter().enumerate() {
            if !searchable(&row.kind) {
                continue;
            }
            let haystack = row
                .cells
                .get(TITLE_COLUMN)
                .map(|cell| cell.to_ascii_lowercase())
                .unwrap_or_default();
            let found = match self.mode {
                SearchMode::Fuzzy => fuzzy_match(&haystack, &needle),
                SearchMode::Substring => haystack.contains(&needle),
                SearchMode::Regex => regex
                    .as_ref()
                    .is_some_and(|regex| regex.is_match(&haystack)),
            };
            if found {
                self.matches.push(index);
            }
        }

        self.targets = self
            .matches
            .iter()
            .filter_map(|index| jump_target(table, *index))
            .collect();
        // A matched heading and a matched first task under it resolve to the
        // same row, so the list is reduced to the rows themselves.
        self.targets.sort_unstable();
        self.targets.dedup();
    }
}

/// Whether a row kind has text worth searching.
///
/// Project headings are deliberately left out: which projects are in play is
/// the question the project list answers, and a `/` that jumped to a project
/// heading would be answering it twice.
fn searchable(kind: &TaskRowKind) -> bool {
    matches!(kind, TaskRowKind::Task | TaskRowKind::SectionHeader)
}

/// Where the cursor goes for a matched row.
///
/// A task row is its own answer. A section heading resolves to the first task
/// under it and stops at the next heading, so a match on an empty section
/// sends the cursor nowhere rather than into the section below.
fn jump_target(table: &TaskTableModel, index: usize) -> Option<usize> {
    if table.rows.get(index)?.kind.is_task() {
        return Some(index);
    }
    table.rows.iter().enumerate().skip(index + 1).find_map(
        |(row_index, row)| match row.kind {
            TaskRowKind::Task => Some(Some(row_index)),
            TaskRowKind::SectionHeader | TaskRowKind::ProjectHeader => Some(None),
            TaskRowKind::SectionSpacer | TaskRowKind::ProjectSeparator => None,
        },
    )?
}

#[cfg(test)]
mod tests {
    use super::{SearchMode, TaskSearchState};
    use crate::domain::{TaskRow, TaskTableModel};

    /// A table of: project heading, section heading, two tasks, a second
    /// section heading with one task, and an empty section at the end.
    fn table() -> TaskTableModel {
        TaskTableModel {
            columns: vec!["Title".to_string()],
            custom_field_columns: Vec::new(),
            rows: vec![
                TaskRow::project_header("Northwind", 1),
                TaskRow::section_header("Shipment", 1),
                TaskRow::task("t1", vec!["Ship the release candidate".to_string()]),
                TaskRow::task("t2", vec!["Confirm carrier pickup".to_string()]),
                TaskRow::section_spacer(1),
                TaskRow::section_header("Study Kit Design", 1),
                TaskRow::task("t3", vec!["Draft the kit insert".to_string()]),
                TaskRow::section_spacer(1),
                TaskRow::section_header("Shipping holds", 1),
            ],
        }
    }

    fn searched(query: &str, mode: SearchMode) -> TaskSearchState {
        let mut search = TaskSearchState::new(None);
        search.set_mode(mode);
        for ch in query.chars() {
            search.push_char(ch);
        }
        search.refresh(&table());
        search
    }

    #[test]
    fn a_fuzzy_query_matches_a_subsequence_of_the_title() {
        let search = searched("shprc", SearchMode::Fuzzy);

        assert!(search.matched_row(2), "Ship the release candidate");
        assert!(!search.matched_row(3));
        assert_eq!(search.target_count(), 1);
    }

    #[test]
    fn a_section_name_matches_and_sends_the_cursor_to_its_first_task() {
        let search = searched("shipment", SearchMode::Substring);

        // The heading is picked out where it is...
        assert!(search.matched_row(1));
        // ...and the row `n` visits is the first task under it.
        assert_eq!(search.step(None, 1), Some(2));
    }

    /// The section at the end has no tasks, so there is nothing to send the
    /// cursor to — and the heading is still worth highlighting.
    #[test]
    fn a_matched_empty_section_is_highlighted_and_visited_by_nothing() {
        let search = searched("holds", SearchMode::Substring);

        assert!(search.matched_row(8));
        assert_eq!(search.target_count(), 0);
        assert_eq!(search.step(None, 1), None);
    }

    /// A heading and the task under it can both match, and they resolve to
    /// one row: `n` must not stop twice in the same place.
    #[test]
    fn a_heading_and_its_first_task_are_one_stop() {
        let mut search = TaskSearchState::new(None);
        search.set_mode(SearchMode::Substring);
        for ch in "ship".chars() {
            search.push_char(ch);
        }
        search.refresh(&table());

        // "Shipment", "Ship the release candidate", and "Shipping holds".
        assert!(search.matched_row(1));
        assert!(search.matched_row(2));
        assert!(search.matched_row(8));
        assert_eq!(search.target_count(), 1);
    }

    #[test]
    fn matching_is_case_insensitive_in_every_mode() {
        for mode in [SearchMode::Fuzzy, SearchMode::Substring, SearchMode::Regex] {
            let search = searched("SHIP THE", mode);
            assert!(search.matched_row(2), "{mode:?} missed the title");
        }
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        let search = searched("the", SearchMode::Substring);

        // "Ship the release candidate" and "Draft the kit insert".
        assert_eq!(search.target_count(), 2);
        assert_eq!(search.step(Some(2), 1), Some(6));
        assert_eq!(search.step(Some(6), 1), Some(2));
        assert_eq!(search.step(Some(2), -1), Some(6));
        assert_eq!(search.step(Some(6), -1), Some(2));
    }

    /// The cursor can be moved off the matches with `j`, and `n` from there
    /// goes to the next one below rather than back to the first.
    #[test]
    fn stepping_from_a_row_that_is_not_a_match_moves_to_the_next_one() {
        let search = searched("the", SearchMode::Substring);

        assert_eq!(search.step(Some(3), 1), Some(6));
        assert_eq!(search.step(Some(3), -1), Some(2));
        assert_eq!(search.position_of(3), None);
        assert_eq!(search.position_of(6), Some(2));
    }

    #[test]
    fn the_first_target_is_the_one_at_or_after_the_origin_and_wraps() {
        let search = searched("the", SearchMode::Substring);

        assert_eq!(search.first_target(Some(0)), Some(2));
        assert_eq!(search.first_target(Some(3)), Some(6));
        // Nothing below the origin, so it comes back to the top.
        assert_eq!(search.first_target(Some(7)), Some(2));
    }

    #[test]
    fn an_empty_query_finds_nothing_rather_than_everything() {
        let search = searched("", SearchMode::Fuzzy);

        assert_eq!(search.target_count(), 0);
        assert!(!search.matched_row(2));
        assert!(search.error().is_none());
    }

    #[test]
    fn an_unparseable_pattern_reports_itself_and_matches_nothing() {
        let search = searched("ship(", SearchMode::Regex);

        assert!(search.error().is_some());
        assert_eq!(search.target_count(), 0);
        assert!(!search.matched_row(2));
    }

    /// A project heading is not searched, so the project name is not a way to
    /// move the cursor.
    #[test]
    fn a_project_heading_is_not_searched() {
        let search = searched("northwind", SearchMode::Substring);

        assert!(!search.matched_row(0));
        assert_eq!(search.target_count(), 0);
    }
}
