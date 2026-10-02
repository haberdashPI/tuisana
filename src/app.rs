use crate::{
    asana::{AsanaClient, TaskQuery, TaskTarget, TaskWrite, MAX_BATCH_ACTIONS},
    config::{Config, Mode, NamedFilterSet, TopPaneState, ViewConfig},
    domain::{ParentEdit, Project, ProjectEdit, ProjectKind, Section, TaskEdit},
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex,
    },
    thread,
};

pub mod autocomplete;
pub mod calendar;
pub mod gantt;
pub mod project_list;
pub mod task;
pub mod task_edit;
pub mod text_edit;

use self::gantt::MoveTo;
use self::task::DraftCommit;
use self::project_list::ProjectListState;
use self::task::TaskState;
use self::text_edit::TextCut;

/// Internal state for the top pane's size and transient maximize/minimize mode.
///
/// The pane is shared between the project list and the filter view, so this
/// type remembers the preferred height and whether the pane is temporarily
/// hidden or expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneMode {
    Normal,
    Minimized,
    Maximized,
}

/// Tracks the preferred height of the top pane and how it should be rendered.
///
/// The project list and filter panel both live in this region. The app uses
/// this state to remember the user's preferred split and to support temporary
/// minimize/maximize toggles.
#[derive(Clone, Debug)]
pub struct PaneSizeState {
    preferred: u16,
    saved_preferred: u16,
    mode: PaneMode,
}

impl Default for PaneSizeState {
    fn default() -> Self {
        Self {
            preferred: 11,
            saved_preferred: 11,
            mode: PaneMode::Normal,
        }
    }
}

impl PaneSizeState {
    pub fn preferred(&self) -> u16 {
        self.preferred
    }

    pub fn is_minimized(&self) -> bool {
        matches!(self.mode, PaneMode::Minimized)
    }

    pub fn is_maximized(&self) -> bool {
        matches!(self.mode, PaneMode::Maximized)
    }

    pub fn is_normal(&self) -> bool {
        matches!(self.mode, PaneMode::Normal)
    }

    pub fn resize(&mut self, delta: i16) {
        let updated = (self.preferred as i16 + delta).max(4) as u16;
        self.preferred = updated;
        if matches!(self.mode, PaneMode::Normal) {
            self.saved_preferred = self.preferred;
        }
    }

    pub fn minimize(&mut self) {
        if !matches!(self.mode, PaneMode::Minimized) {
            self.saved_preferred = self.preferred;
            self.mode = PaneMode::Minimized;
        }
    }

    pub fn maximize(&mut self) {
        if !matches!(self.mode, PaneMode::Maximized) {
            self.saved_preferred = self.preferred;
            self.mode = PaneMode::Maximized;
        }
    }

    pub fn restore(&mut self) {
        if !matches!(self.mode, PaneMode::Normal) {
            self.preferred = self.saved_preferred.max(4);
            self.mode = PaneMode::Normal;
        }
    }

    pub fn actual_height(&self, available: u16, min_height: u16) -> u16 {
        let min_height = min_height.max(1);
        let max_height = available.max(min_height);
        match self.mode {
            PaneMode::Normal => self.preferred.clamp(min_height, max_height),
            PaneMode::Minimized => 0,
            PaneMode::Maximized => max_height,
        }
    }
}

/// Central application state and coordinator.
///
/// `App` owns the project list state, task state, bind mode, pane
/// sizing, and asynchronous task loading. The runtime and tests drive this
/// type directly so the UI stays thin and the behavior remains easy to verify.
#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
    pub tasks: TaskState,
    mode: Mode,
    panel_size: PaneSizeState,
    client: C,
    /// Monotonic tag for the current async task-data request.
    ///
    /// Each new task-data request increments this counter. Worker messages carry the
    /// generation they were started under so `poll_task_data` can ignore stale
    /// results from an earlier request after the user has triggered a newer one.
    task_data_generation: u64,
    /// Receiver for the in-flight async task-data worker, if one is active.
    ///
    /// The app keeps the receiver here so the main loop can poll for partial
    /// results, completion, or failure without blocking the UI.
    task_data_receiver: Option<Receiver<TaskDataMessage>>,
    /// Results from in-flight task writes.
    ///
    /// A permanent pair rather than one receiver per batch: `d` on twelve
    /// tasks is twelve requests, and the next edit must not have to wait for
    /// them. Each message names its task, so no generation counter is needed —
    /// a late reply to a superseded edit is reconciled by gid, not discarded.
    task_edit_events: (Sender<TaskEditMessage>, Receiver<TaskEditMessage>),
    /// Writes sent, still outstanding, and failed in the current burst.
    ///
    /// Counted rather than reported one by one: twelve failures is one notice,
    /// `could not update 3 of 12: …`, not three that overwrite each other.
    /// Reset once the last reply of a burst has landed.
    task_edits_sent: usize,
    task_edits_outstanding: usize,
    task_edit_failures: usize,
    /// The last write error, which is what the notice quotes.
    task_edit_error: Option<String>,
    /// The logged-in user's gid, for resolving `me` in an assignee edit.
    current_user_gid: Option<String>,
    /// The workspace directory, fetched once and kept for the session.
    ///
    /// `None` until the first editor that needs it opens: most sessions never
    /// reassign anything, and a request nobody needed is a request not worth
    /// making at startup. A failure leaves it `Some(empty)` rather than
    /// `None`, so a workspace that will not answer is asked exactly once.
    people: Option<Vec<(String, String)>>,
    /// Whether the saved project selection has been put back yet.
    ///
    /// The panes and the bound filter set are restored in [`App::new`], but
    /// the selection needs projects to exist, so it waits for the first
    /// [`App::load_projects`]. `refresh` goes through that same call and must
    /// not overwrite what the user has selected since.
    view_restored: bool,
    /// The entry and project selection the projects half of the write-through
    /// last agreed on.
    ///
    /// The name is `None` for the scratch entry, which is where an unnamed
    /// panel's selection goes — so moving between a named set and no set at
    /// all counts as the binding changing, and re-baselines like any other
    /// load.
    ///
    /// The fields half has a dirty flag the filter editor sets; the project
    /// list has none and should not grow one. This is the baseline instead:
    /// the selection as it stood when the binding was established, so a
    /// *change* to it is what writes, rather than any disagreement with the
    /// file. That difference is what keeps §2.2's rule — a gid the workspace
    /// no longer returns is dropped from the live selection on load, and
    /// staying silent about it is what leaves it on disk until the selection
    /// is deliberately edited.
    ///
    /// `None` when the panel is unbound, and reset whenever the name changes,
    /// which is how a load, a save under a new name, and the startup restore
    /// all re-baseline without each having to remember to.
    bound_projects: Option<(Option<String>, Vec<String>)>,
    /// The mode the sidebar prompt interrupted, so answering it goes back
    /// there.
    ///
    /// The sets keys work in the project view as well as the filter view, and
    /// a `y` at a confirmation must not swap the project list away under the
    /// key that was pressed in it.
    prompt_return_mode: Mode,
    /// The version-1 config this session has yet to be allowed to rewrite.
    ///
    /// `App` state rather than config state: it is a question on screen, and
    /// the file on disk is untouched until it is answered. While it is set,
    /// the prompt owns every key and nothing may write the config.
    pending_migration: Option<PendingMigration>,
    /// The bulk edit waiting on a `y`.
    ///
    /// Holding the writes here rather than sending them is the whole point:
    /// nothing is applied, locally or remotely, until the count on screen has
    /// been agreed to. `n` drops it and the table never moved.
    pending_bulk_edit: Option<PendingBulkEdit>,
    /// The threads that send batched writes.
    ///
    /// `None` until the first write of the session: most of a session is
    /// reading, and two threads blocked on a condvar for a user who never
    /// edits anything are two threads that should never have been started.
    write_pool: Option<WritePool>,
}

/// A bulk edit that has been composed but not sent.
///
/// The writes are already resolved — the editor is closed and the value
/// decided — so answering `y` is a dispatch and nothing more. That ordering
/// matters: resolving them after the question would mean the dialog could
/// name a count the commit then disagreed with.
#[derive(Clone, Debug)]
struct PendingBulkEdit {
    /// What it will do, in the dialog's words. One line, no count: the count
    /// is the title, because it is the thing being agreed to.
    summary: String,
    /// The writes, already chunked by `enqueue_writes` when they go.
    writes: Vec<PendingEdit>,
    /// The mode to go back to, whichever way it is answered.
    ///
    /// Carried rather than assumed: `d` is pressed in task mode and `enter`
    /// closes a cell editor, and both have to land back where they were.
    return_mode: Mode,
}

impl PendingBulkEdit {
    fn count(&self) -> usize {
        self.writes.len()
    }
}

/// One phrase naming what a run of writes will do.
///
/// Every write in a bulk edit is the same change to a different task, so the
/// first one describes the lot — with one exception worth spelling out: a
/// projects edit resolves per task, so "be in these projects" becomes an
/// `add` for one row and a `remove` for another. When the writes disagree,
/// the summary counts them instead of picking one and misreporting the rest.
fn bulk_edit_summary(writes: &[PendingEdit], column: Option<&str>) -> String {
    let phrases = writes
        .iter()
        .map(|write| match write {
            PendingEdit::Field(edit) => edit.field.summary(column),
            PendingEdit::Project(edit) => edit.summary(),
            PendingEdit::Parent(edit) => edit.summary(),
        })
        .collect::<std::collections::BTreeSet<_>>();

    let mut phrases = phrases.into_iter();
    match (phrases.next(), phrases.next()) {
        (Some(only), None) => only,
        (Some(_), Some(_)) => format!("make {} changes", writes.len()),
        _ => String::new(),
    }
}

/// A version-1 config waiting for permission to be rewritten.
#[derive(Clone, Debug)]
struct PendingMigration {
    /// The version the file on disk was written in.
    ///
    /// Two versions can need migrating now, and they do not need the same
    /// changes — so the window has to say which ones are about to happen
    /// rather than describe a fixed pair.
    from_version: f64,
    /// The file `y` would copy the config to.
    ///
    /// Resolved up front so the window can name the file it will actually
    /// write: an existing `tuisana.backup.toml` is not clobbered, and the
    /// choice has to be made with the right name on screen.
    backup_path: PathBuf,
}

/// One write still in flight, in the shape its rollback needs.
///
/// A field edit and a membership change go to different endpoints and come
/// back with different things, but they are counted, reported, and rolled
/// back as one burst — twelve failures are one notice whichever kind they
/// were.
#[derive(Clone, Debug)]
enum PendingEdit {
    Field(TaskEdit),
    Project(ProjectEdit),
    Parent(ParentEdit),
}

impl PendingEdit {
    fn gid(&self) -> &str {
        match self {
            Self::Field(edit) => &edit.gid,
            Self::Project(edit) => &edit.gid,
            Self::Parent(edit) => &edit.gid,
        }
    }

    /// The same write, stripped of what it needs to undo itself.
    ///
    /// The client has no business with the rollback — that is what keeps
    /// `TaskWrite` free of a `previous` field it would never read.
    fn as_write(&self) -> TaskWrite {
        match self {
            Self::Field(edit) => TaskWrite::Field {
                gid: edit.gid.clone(),
                edit: edit.field.clone(),
            },
            Self::Project(edit) => TaskWrite::Project(edit.clone()),
            Self::Parent(edit) => TaskWrite::Parent {
                gid: edit.gid.clone(),
                parent_gid: edit.parent_gid.clone(),
            },
        }
    }
}

/// How many worker threads send batches.
///
/// Two, because the real limit is Asana's concurrent-action ceiling and
/// `WriteThrottle` is what enforces it: at ten actions a batch, two workers
/// already have more in flight than the budget allows, and a third would only
/// queue inside `acquire`. Two is enough for one worker to be building the
/// next chunk's reads while the other is waiting on the wire.
const WRITE_WORKERS: usize = 2;

/// The threads that send batched writes, and the queue they take from.
///
/// One pool for the session rather than a thread per edit. The old shape —
/// `thread::spawn` once per task — meant selecting forty rows and pressing
/// `d` opened forty sockets at once, which is nearly three times Asana's
/// concurrent-write ceiling and the quickest way to be told `429`. Here the
/// work is chunked to [`MAX_BATCH_ACTIONS`] and handed to a fixed pool, and
/// `WriteThrottle` inside the client paces what the pool is allowed to send.
///
/// The workers outlive every burst and block on the condvar between them, so a
/// second bulk edit costs no thread spawns. They are never joined: there is
/// nothing to flush at exit that `settle_task_edits` has not already waited
/// for.
#[derive(Debug)]
struct WritePool {
    queue: Arc<WriteQueue>,
}

/// Chunks waiting for a worker.
#[derive(Debug)]
struct WriteQueue {
    chunks: Mutex<VecDeque<Vec<PendingEdit>>>,
    queued: Condvar,
}

impl WritePool {
    /// Starts the pool. Each worker gets its own client clone and a handle on
    /// the one reply channel.
    fn new<C: AsanaClient + Clone + Send + 'static>(
        client: &C,
        sender: &Sender<TaskEditMessage>,
    ) -> Self {
        let queue = Arc::new(WriteQueue {
            chunks: Mutex::new(VecDeque::new()),
            queued: Condvar::new(),
        });

        for _ in 0..WRITE_WORKERS {
            let queue = Arc::clone(&queue);
            let client = client.clone();
            let sender = sender.clone();
            thread::spawn(move || loop {
                let chunk = {
                    let mut chunks = match queue.chunks.lock() {
                        Ok(chunks) => chunks,
                        Err(_) => return,
                    };
                    loop {
                        if let Some(chunk) = chunks.pop_front() {
                            break chunk;
                        }
                        chunks = match queue.queued.wait(chunks) {
                            Ok(chunks) => chunks,
                            Err(_) => return,
                        };
                    }
                };

                let writes = chunk.iter().map(PendingEdit::as_write).collect::<Vec<_>>();
                let results = client.write_tasks(&writes);
                // Zipped, so a client that answered with the wrong number of
                // results loses the extras rather than panicking. Every reply
                // decrements the outstanding count, which is why a missing one
                // would wedge `settle_task_edits` — and why `write_tasks`
                // promises one result per write.
                for (edit, result) in chunk.into_iter().zip(results) {
                    if sender.send(TaskEditMessage { edit, result }).is_err() {
                        return;
                    }
                }
            });
        }

        Self { queue }
    }

    /// Queues one chunk and wakes a worker.
    fn submit(&self, chunk: Vec<PendingEdit>) {
        let Ok(mut chunks) = self.queue.chunks.lock() else {
            return;
        };
        chunks.push_back(chunk);
        drop(chunks);
        self.queue.queued.notify_one();
    }
}

/// What a committed draft turned into.
///
/// Two shapes because `enter` on a draft means two different writes and the
/// caller has already stopped caring which: the draft knows.
enum Created {
    /// Boxed: a `TaskDto` is an order of magnitude larger than a `Section`,
    /// and this enum only ever carries one of them across one match.
    Task(Box<crate::asana::dto::TaskDto>),
    Section(Section),
}

/// One finished write, on its way back to the main thread.
struct TaskEditMessage {
    edit: PendingEdit,
    /// The server's `modified_at` on success, when the endpoint reports one.
    ///
    /// `addProject` answers with nothing worth keeping, so a membership
    /// change succeeds with `None` — and takes no timestamp with it.
    result: Result<Option<String>>,
}

/// One of the shared text motions, for [`App::move_text_caret`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextMotion {
    WordBack,
    WordForward,
    Start,
    End,
}

/// Internal message sent back from the task-data worker thread.
///
/// The app uses this to merge task data incrementally and to know when a request
/// has completed or failed.
struct TaskDataMessage {
    generation: u64,
    project_id: Option<String>,
    query: TaskQuery,
    result: Result<crate::app::task::TaskDataset>,
    done: bool,
}

impl<C: AsanaClient + Clone + Send + 'static> App<C> {
    pub fn new(config: Config, client: C) -> Self {
        let mut tasks = TaskState::new();
        tasks.apply_gantt_config(&config.gantt);
        let mut app = Self {
            config,
            projects: ProjectListState::new(),
            tasks,
            mode: Mode::Project,
            panel_size: PaneSizeState::default(),
            client,
            task_data_generation: 0,
            task_data_receiver: None,
            task_edit_events: mpsc::channel(),
            task_edits_sent: 0,
            task_edits_outstanding: 0,
            task_edit_failures: 0,
            task_edit_error: None,
            current_user_gid: None,
            people: None,
            view_restored: false,
            bound_projects: None,
            prompt_return_mode: Mode::Filter,
            pending_migration: None,
            pending_bulk_edit: None,
            write_pool: None,
        };
        app.apply_view_config();
        // Last, so nothing above it can have written the file: a version-1
        // config is read, migrated in memory, and then left alone until the
        // user says what to do with it.
        if app.config.needs_migration() {
            app.pending_migration = Some(PendingMigration {
                from_version: app.config.migrated_from_version(),
                backup_path: backup_path_for(app.config.source_path()),
            });
        }
        app
    }

    /// The file the migration prompt would back the config up to.
    ///
    /// `None` when no prompt is up, which is what the renderer reads to
    /// decide whether to draw it at all.
    pub fn pending_migration_backup(&self) -> Option<&Path> {
        self.pending_migration
            .as_ref()
            .map(|pending| pending.backup_path.as_path())
    }

    /// The version the config on disk is in, while the prompt is up.
    pub fn pending_migration_from_version(&self) -> Option<f64> {
        self.pending_migration
            .as_ref()
            .map(|pending| pending.from_version)
    }

    /// The migration prompt's three keys.
    ///
    /// The prompt owns every key it is shown, so this consumes the event
    /// whatever it was: a stray `j` behind a question about rewriting a
    /// hand-written file must not move a cursor nobody can see.
    fn handle_migration_input(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Option<AppCommand> {
        use crossterm::event::KeyCode;

        let pending = self.pending_migration.clone()?;

        match event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // A copy, not a re-serialization: the backup keeps the user's
                // comments, key order, and formatting, none of which survive
                // `toml::to_string_pretty`.
                let source = self.config.source_path().map(Path::to_path_buf);
                if let Some(source) = source {
                    if let Err(err) = std::fs::copy(&source, &pending.backup_path) {
                        debug_log(&format!("config backup failed: {err}"));
                        self.tasks
                            .set_edit_notice(format!("could not write the backup: {err}"));
                        return None;
                    }
                }
                self.finish_migration();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.finish_migration(),
            // Nothing is written, and the prompt returns next time. A user
            // who would rather edit the file by hand can.
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                return Some(AppCommand::Quit)
            }
            _ => {}
        }
        None
    }

    /// Lifts the write gate and puts the migrated config on disk.
    fn finish_migration(&mut self) {
        self.pending_migration = None;
        self.config.clear_migration();
        self.write_config_or_report();
    }

    /// Puts the panes and the bound filter set back the way `[view]` left them.
    ///
    /// Everything here is state the app can hold before it has spoken to
    /// Asana. The project selection cannot — it needs a project list to be
    /// filtered against — so it waits for [`Self::load_projects`].
    fn apply_view_config(&mut self) {
        let view = self.config.view.clone();

        // Before the panes, because binding the panel is what the first fetch
        // reads its query from: a saved `due` filter narrows the window
        // pushed down to Asana, and the fetch is one `ensure_task_data` away.
        if let Some(entry) = view.filter_set.as_deref().and_then(|name| {
            self.config
                .filter_sets
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(name))
                .cloned()
        }) {
            self.tasks.filter_sets_load(&entry.name, &entry.sets);
        }

        self.tasks.set_recent_pane_enabled(view.recent);
        self.tasks.set_filter_sets_sidebar(view.filter_sidebar);

        // The top pane's content is decided by the mode, so restoring the
        // filter panel means starting in filter mode. Only the panel, not the
        // focus: which pane the keys were going to is where you were looking,
        // and `[view]` records what was open.
        if view.filters {
            self.set_filter_mode();
        } else if view.tasks {
            self.tasks.set_visible(true);
        }

        match view.top_pane {
            TopPaneState::Normal => self.panel_size.restore(),
            TopPaneState::Maximized => self.panel_size.maximize(),
            TopPaneState::Minimized => {
                // A minimized top pane leaves nothing for project or filter
                // mode to drive, which is why `{` moves to the table as it
                // minimizes. A restore that skipped that would put the cursor
                // in a pane that is not on screen.
                self.set_task_mode();
                self.panel_size.minimize();
            }
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects
            .load_with_visibility(&self.client, &self.config.project_visibility)?;
        // The "assigned to me" row is best-effort: if the client can't resolve
        // who's logged in (e.g. the token lacks permission, or a test double
        // has no fake user configured), just omit the row instead of failing
        // project loading entirely.
        let assigned_to_me = match self.client.current_user_gid() {
            Ok(gid) => {
                self.current_user_gid = Some(gid.clone());
                // The filter's `me` resolves at match time, far from here, so
                // the task pane keeps its own copy.
                self.tasks.set_current_user_gid(Some(gid.clone()));
                Some(Project::assigned_to_me(gid))
            }
            Err(err) => {
                debug_log(&format!("current_user_gid unavailable: {err}"));
                None
            }
        };
        self.projects.set_assigned_to_me(assigned_to_me);
        // After the "assigned to me" row, which is synthetic and would not be
        // among the live gids a moment earlier — saving it and then dropping
        // it on the way back in would be the one selection the view could
        // never keep.
        if !self.view_restored {
            self.view_restored = true;
            self.projects.restore_selection(&self.restored_selection());
            // The baseline is whatever the restore landed on: that is what
            // the file already says, so nothing is written until the user
            // moves it. Set here rather than left for the first settled burst
            // because the first key of a session is often a selection change,
            // and a baseline that was still unset would swallow it.
            self.bound_projects = Some((
                self.tasks.filter_set_loaded_name().map(str::to_string),
                self.projects.selection_for_filter_set(),
            ));
            // After `view_restored`, so the gate can be judged at all: the
            // panes were restored before Asana was spoken to, and a
            // `filters = true` that turns out to have nothing selected has to
            // give way now rather than opening onto an empty panel.
            self.enforce_project_gate();
            // A restored selection with the task pane open is a request for
            // those tasks; nothing else would have asked for them until the
            // user pressed a key.
            self.ensure_task_data();
        }
        Ok(())
    }

    /// Start the asynchronous task-data request for the currently selected projects.
    pub fn request_task_data(&mut self) -> Result<()> {
        self.start_task_data_fetch();
        Ok(())
    }

    /// Reload the project list and force a fresh task fetch for the current
    /// selection.
    ///
    /// A plain `ensure_task_data` call would see the selection already
    /// "covered" by the cache and skip fetching, so a manual refresh needs to
    /// invalidate that cache first — otherwise pressing refresh on a project
    /// that's already loaded looks like it does nothing.
    pub fn refresh(&mut self) -> Result<()> {
        self.load_projects()?;
        self.tasks.invalidate_cache();
        if self.task_data_receiver.is_some() {
            self.task_data_generation = self.task_data_generation.wrapping_add(1);
        }
        if self.tasks.visible() {
            self.start_task_data_fetch();
        } else {
            self.tasks
                .mark_out_of_date("refreshed; switch to task view to reload");
        }
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.effective_bindings())
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn panel_size(&self) -> &PaneSizeState {
        &self.panel_size
    }

    /// Shared setup for mode transitions that use the top pane: restores the
    /// pane if it was minimized and closes any active project search.
    fn prepare_mode_switch(&mut self) {
        if self.panel_size.is_minimized() {
            self.restore_top_pane();
        }
        if self.projects.search_active() {
            self.projects.end_search();
        }
    }

    fn set_project_mode(&mut self) {
        self.prepare_mode_switch();
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        self.mode = Mode::Project;
    }

    fn set_project_search_mode(&mut self) {
        if self.panel_size.is_minimized() {
            self.restore_top_pane();
        }
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        self.mode = Mode::ProjectSearch;
    }

    /// Whether the filter view is closed off because nothing is selected.
    ///
    /// A filter panel with no projects is a question with no subject: its
    /// rows are built from the loaded projects' custom fields, so there is
    /// not even a full set of fields to fill in. Since version 3 the
    /// selection is part of the set, so the answer is to pick projects first
    /// — which is what the project view is for.
    ///
    /// Gated on the selection having been resolved at all: before the first
    /// [`Self::load_projects`] nothing is selected because nothing has been
    /// loaded yet, and refusing then would stop `[view].filters` from ever
    /// being restored.
    fn project_gate_closed(&self) -> bool {
        self.view_restored && self.projects.selected_count() == 0
    }

    /// Puts the keys back in the project view when the filter view has become
    /// unreachable under them.
    ///
    /// `n` clears the selection and is pressed *in* the filter view, and a
    /// load can bring an entry that names no projects, so the gate has to be
    /// enforced after the fact as well as refused up front.
    fn enforce_project_gate(&mut self) {
        if !self.project_gate_closed() {
            return;
        }
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        if matches!(
            self.mode,
            Mode::Filter | Mode::FilterEdit | Mode::Calendar | Mode::FilterSetName
        ) {
            self.mode = Mode::Project;
        }
    }

    fn set_filter_mode(&mut self) {
        if self.project_gate_closed() {
            self.tasks
                .set_edit_notice("No projects selected.\nSelect projects, then press f.");
            self.set_project_mode();
            return;
        }
        self.prepare_mode_switch();
        self.mode = Mode::Filter;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        if self.tasks.filter_panel_editing() {
            self.tasks.filter_edit_done();
        }
    }

    fn set_filter_edit_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::FilterEdit;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        if !self.tasks.filter_panel_editing() {
            self.begin_filter_field_edit();
        }
    }

    /// Starts editing the selected filter row, with its candidates when the
    /// row is one that has any.
    ///
    /// The directory is fetched only for the row that can use it: every other
    /// row is free text, and a request to open one of those would be a
    /// request for nothing.
    fn begin_filter_field_edit(&mut self) {
        if !self.tasks.filter_row_completes() {
            self.tasks.filter_edit_begin();
            return;
        }
        let context = self.edit_context();
        let candidates = self.tasks.people_candidates(&context);
        self.tasks.filter_edit_begin_with(candidates);
    }

    /// Resolves whatever the panel's completion editor still holds.
    fn finish_filter_completion(&mut self) {
        if let Some(message) = self.tasks.filter_autocomplete_commit() {
            self.tasks.set_edit_notice(message);
        }
    }

    /// `tab`: completes the prefix in whichever editor is open.
    fn complete_candidate(&mut self, delta: i32) {
        let (open, completed) = match self.tasks.cell_edit_is_complete() {
            true => (true, self.tasks.cell_edit_complete(delta)),
            false => (
                self.tasks.filter_autocomplete_open(),
                self.tasks.filter_complete(delta),
            ),
        };
        // A prefix that matches nothing is worth saying so: the key looked
        // like it did nothing, and the reason is that there is nothing to do.
        if open && !completed {
            self.tasks.set_edit_notice("nothing to complete");
        }
    }

    /// `ctrl-n` / `ctrl-p`: walks the candidate list without taking any of it.
    ///
    /// The other half of the pair `tab` is. `enter` takes whatever the
    /// highlight landed on, which keeps `tab` meaning what it always meant —
    /// type, `tab` until it reads right, `enter` — and adds the other idiom
    /// beside it rather than replacing it.
    fn highlight_candidate(&mut self, delta: i32) {
        if !self.tasks.move_completion_highlight(delta) {
            self.tasks.set_edit_notice("nothing to complete");
        }
    }

    /// Enters the date picker. The picker owns the whole date edit, so the
    /// filter panel's own text-edit flag stays off.
    fn set_calendar_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::Calendar;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    /// Enters gantt mode, drawing the chart. Follows `set_filter_mode`: the
    /// mode and the thing it drives are turned on together.
    fn set_gantt_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::Gantt;
        self.tasks.set_visible(true);
        self.tasks.gantt_mut().set_visible(true);
        // Same pane, same rows, same rule as `set_task_mode`: the top pane
        // keeps whatever it was holding.
    }

    /// Enters the mode the open cell editor reads its keys in.
    ///
    /// A date cell is edited on the calendar, which already owns a mode and a
    /// full set of keys; everything else types.
    fn set_task_edit_mode(&mut self) {
        self.mode = match self.tasks.cell_edit_owns_calendar() {
            true => Mode::Calendar,
            false => Mode::ColumnEdit,
        };
    }

    /// Whether the keys are currently driving the filter panel.
    ///
    /// Not the same question as whether the panel is *visible*: it stays up
    /// top when the keys move to the table, so the panel can be on screen
    /// with nothing going to it. Calendar mode is the one mode both panes can
    /// be in — a filter date and a task cell open the same picker — and the
    /// editor that owns it is what tells them apart.
    pub fn filter_panel_focused(&self) -> bool {
        match self.mode {
            Mode::Filter | Mode::FilterEdit | Mode::FilterSetName => true,
            Mode::Calendar => !self.tasks.cell_edit_owns_calendar(),
            _ => false,
        }
    }

    fn set_task_mode(&mut self) {
        if self.projects.search_active() {
            self.projects.end_search();
        }
        self.mode = Mode::Task;
        self.tasks.set_visible(true);
        // No binding reaches here from filter-edit mode — that mode has no
        // global fallback, so its letters type — and the two callers that
        // close the panel clear the flag on the way. This is what keeps an
        // open field from taking the table's keys if a third one appears.
        if self.tasks.filter_panel_editing() {
            self.tasks.filter_edit_done();
        }
        // The filter panel is left where it is. Moving the keys to the table
        // is not a request to put the project list back: the filters you just
        // built are the thing you are reading the table against, and `f` and
        // `p` are both one keystroke away when you do want them gone.
    }

    /// Enters edit mode, taking the keys that change which rows exist.
    ///
    /// Follows `set_gantt_mode`: the same pane, the same rows, the same
    /// cursor, and the top pane keeps whatever it was holding.
    fn set_edit_mode(&mut self) {
        if self.projects.search_active() {
            self.projects.end_search();
        }
        self.mode = Mode::Edit;
        self.tasks.set_visible(true);
        if self.tasks.filter_panel_editing() {
            self.tasks.filter_edit_done();
        }
    }

    /// Leaves edit mode, or clears the deletion marks when there are any.
    ///
    /// One key, one sentence: back out of whatever is pending.
    fn edit_cancel(&mut self) {
        if self.tasks.clear_deletion_marks() {
            return;
        }
        self.set_task_mode();
    }

    /// Opens a draft task, committing any draft already open.
    ///
    /// A second `i` while one is open commits the first, which is what makes
    /// `i` … `enter` `i` … a run of new tasks and `i` … `i` … the same run
    /// one keystroke shorter.
    fn insert_task(&mut self, as_subtask: bool) {
        if self.tasks.draft_gid().is_some() {
            self.commit_draft();
            // A refusal left the draft on screen with its title intact;
            // opening a second one over it would lose exactly the text the
            // refusal was protecting.
            if self.tasks.draft_gid().is_some() {
                return;
            }
        }
        let context = self.edit_context();
        match self.tasks.begin_draft_task(as_subtask, &context) {
            Ok(()) => self.mode = Mode::ColumnEdit,
            Err(message) => self.tasks.set_edit_notice(message),
        }
    }

    /// Opens a draft section after the cursor's.
    fn insert_section(&mut self) {
        if self.tasks.draft_gid().is_some() {
            self.commit_draft();
            if self.tasks.draft_gid().is_some() {
                return;
            }
        }
        let context = self.edit_context();
        match self.tasks.begin_draft_section(&context) {
            Ok(()) => self.mode = Mode::ColumnEdit,
            Err(message) => self.tasks.set_edit_notice(message),
        }
    }

    /// Sends whatever the open draft holds, and returns to edit mode.
    ///
    /// Synchronous, unlike the field edits: a create has no local value to
    /// show while the request is in flight — it has a gid it does not know
    /// yet, and every subsequent edit of that row needs it. A failure leaves
    /// the draft and its title on screen, because the alternative is losing
    /// what was just typed to a 403.
    fn commit_draft(&mut self) {
        let Some(commit) = self.tasks.draft_commit() else {
            // An empty name discards the draft with nothing sent, the same
            // as `esc`: nothing happened, so there is nothing to undo.
            self.tasks.cancel_draft();
            self.set_edit_mode();
            return;
        };

        let result = match &commit {
            DraftCommit::Task(task) => self
                .client
                .create_task(task)
                .map(|task| Created::Task(Box::new(task))),
            DraftCommit::Section {
                project_gid,
                name,
                insert_after,
            } => self
                .client
                .create_section(project_gid, name, insert_after.as_deref())
                .map(|section| Created::Section(Section::new(section.gid, section.name))),
        };

        match result {
            Ok(Created::Task(task)) => {
                self.tasks.clear_edit_notice();
                self.tasks.finish_draft_task(*task);
                self.set_edit_mode();
            }
            Ok(Created::Section(section)) => {
                self.tasks.clear_edit_notice();
                self.tasks.finish_draft_section(section);
                self.set_edit_mode();
            }
            Err(err) => {
                debug_log(&format!("create failed: {err}"));
                self.tasks
                    .set_edit_notice(format!("could not create: {err}"));
            }
        }
    }

    /// `X`: deletes the empty section the cursor is in.
    ///
    /// Not part of the mark-and-confirm flow: that exists because deleting
    /// tasks destroys work, and an empty section holds none.
    fn delete_section(&mut self) {
        let context = self.edit_context();
        let target = match self.tasks.section_to_delete(&context) {
            Ok(target) => target,
            Err(message) => {
                self.tasks.set_edit_notice(message);
                return;
            }
        };

        match self.client.delete_section(&target.section_gid) {
            Ok(()) => {
                self.tasks.set_edit_notice(format!("deleted {}", target.section_name));
                self.tasks.remove_section_locally(&target);
            }
            Err(err) => {
                debug_log(&format!("section delete failed: {err}"));
                self.tasks
                    .set_edit_notice(format!("could not delete {}: {err}", target.section_name));
            }
        }
    }

    /// `J` / `K`: moves the task under the cursor one section along.
    ///
    /// Optimistic: the row jumps to its new group on the keystroke and goes
    /// back if the request fails.
    fn move_task_to_section(&mut self, delta: i32) {
        let context = self.edit_context();
        let moved = match self.tasks.section_move(delta, &context) {
            // Off the end in either direction does nothing. There is no "no
            // section" position to move into below the last one.
            Ok(None) => return,
            Ok(Some(moved)) => moved,
            Err(message) => {
                self.tasks.set_edit_notice(message);
                return;
            }
        };

        self.tasks.clear_edit_notice();
        self.tasks.apply_section_move_locally(&moved);
        if let Err(err) = self
            .client
            .add_task_to_section(&moved.section_gid, &moved.gid)
        {
            debug_log(&format!("section move failed: {err}"));
            self.tasks.undo_section_move_locally(&moved);
            self.tasks
                .set_edit_notice(format!("could not move to {}: {err}", moved.section_name));
        }
    }

    /// `enter` in edit mode: deletes everything `x` marked.
    ///
    /// The rows go at confirm time and come back if the request fails, with
    /// the same `could not … N of M` shape the field edits report.
    fn delete_marked_tasks(&mut self) {
        let marked = self.tasks.marked_for_deletion();
        if marked.is_empty() {
            return;
        }

        // Taken out first, so the table reads as the deletion the user
        // confirmed rather than lagging behind the requests.
        let removed = self.tasks.remove_tasks_locally(&marked);
        let mut failed = Vec::new();
        let mut error = None;
        // Batched and throttled like every other bulk write, but sent on this
        // thread rather than through the pool: the rollback below has to put
        // a failed task's whole limb of subtasks back, and that is a decision
        // over the *whole* result rather than one reply at a time. Marking
        // thirty tasks and deleting them is the one place the UI blocks, and
        // it blocks after a confirmation the user is already waiting on.
        for chunk in marked.chunks(MAX_BATCH_ACTIONS) {
            let writes = chunk
                .iter()
                .map(|gid| TaskWrite::Delete { gid: gid.clone() })
                .collect::<Vec<_>>();
            for (gid, result) in chunk.iter().zip(self.client.write_tasks(&writes)) {
                if let Err(err) = result {
                    debug_log(&format!("task delete failed: {err}"));
                    failed.push(gid.clone());
                    error = Some(err.to_string());
                }
            }
        }

        if failed.is_empty() {
            self.tasks.clear_edit_notice();
            return;
        }

        // Only the ones that failed, and whatever went with them: a batch
        // where two of three succeeded must not put all three back. A failed
        // parent takes its whole limb back, however deep, which is why this
        // is a closure over the removed set rather than one hop.
        let mut keep = failed.iter().cloned().collect::<std::collections::HashSet<_>>();
        loop {
            let before = keep.len();
            for record in &removed {
                if record
                    .parent_gid
                    .as_ref()
                    .is_some_and(|parent| keep.contains(parent))
                {
                    keep.insert(record.gid.clone());
                }
            }
            if keep.len() == before {
                break;
            }
        }
        let restore = removed
            .into_iter()
            .filter(|record| keep.contains(&record.gid))
            .collect::<Vec<_>>();
        self.tasks.restore_tasks_locally(restore);
        self.tasks.set_edit_notice(format!(
            "could not delete {} of {}: {}",
            failed.len(),
            marked.len(),
            error.unwrap_or_else(|| "unknown error".to_string()),
        ));
    }

    fn adjust_top_pane_height(&mut self, delta: i16) {
        self.panel_size.resize(delta);
    }

    fn minimize_top_pane(&mut self) {
        self.panel_size.minimize();
    }

    fn maximize_top_pane(&mut self) {
        self.panel_size.maximize();
    }

    fn restore_top_pane(&mut self) {
        self.panel_size.restore();
    }

    fn task_data_needs_refresh(&self) -> bool {
        let targets = self.task_target_projects();
        let query_template = self.tasks.desired_task_query();
        self.task_data_receiver.is_none()
            && !self.tasks.can_serve_query_for_targets(&targets, &query_template)
    }

    fn ensure_task_data(&mut self) {
        if self.tasks.visible() && self.task_data_needs_refresh() {
            self.start_task_data_fetch();
        }
    }

    fn update_task_data_after_action(&mut self, task_targets_before: Option<Vec<String>>) {
        if let Some(before) = task_targets_before {
            let after = self.task_target_project_ids();
            if before != after
                && !matches!(
                    self.tasks.status(),
                    crate::app::task::TaskStatus::Idle
                )
            {
                if self.task_data_receiver.is_some() {
                    self.task_data_generation = self.task_data_generation.wrapping_add(1);
                }
                if self.tasks.visible() {
                    self.start_task_data_fetch();
                } else {
                    self.tasks.mark_out_of_date(
                        "selected projects changed; switch to task view to refresh",
                    );
                }
            }
        }

        if self.tasks.visible() && self.task_data_needs_refresh() {
            self.start_task_data_fetch();
        }
    }

    /// Handle text input when the filter panel is in edit mode.
    ///
    /// `action` is the label-navigation action the keymap resolved for this event,
    /// if any. On a labels field the action is applied; on any other field the key
    /// falls back to pushing its character. This lets label-navigation bindings be
    /// fully configurable while still typing normally on text and date fields.
    fn handle_filter_field_input(
        &mut self,
        event: crossterm::event::KeyEvent,
        action: Option<&Action>,
    ) -> Result<bool> {
        if !self.tasks.filter_panel_editing() {
            return Ok(false);
        }

        use crossterm::event::{KeyCode, KeyModifiers};
        use crate::app::task::TaskFieldFilterKind;

        let is_plain_char = !event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let is_labels = matches!(self.tasks.filter_selected_kind(), Some(TaskFieldFilterKind::Labels));

        if let KeyCode::Backspace = event.code {
            self.tasks.filter_pop_char();
            return Ok(true);
        }

        if is_labels {
            match action {
                Some(Action::FilterMoveLabelLeft) => { self.tasks.filter_move_label_left(); return Ok(true); }
                Some(Action::FilterMoveLabelRight) => { self.tasks.filter_move_label_right(); return Ok(true); }
                Some(Action::FilterCycleLabelDown) => { self.tasks.filter_cycle_label_value(1); return Ok(true); }
                Some(Action::FilterCycleLabelUp) => { self.tasks.filter_cycle_label_value(-1); return Ok(true); }
                Some(Action::FilterAddLabel) => { self.tasks.filter_add_label(); return Ok(true); }
                Some(Action::FilterDeleteLabel) => { self.tasks.filter_delete_label(); return Ok(true); }
                _ => {}
            }
        }

        if is_plain_char {
            if let KeyCode::Char(c) = event.code {
                self.tasks.filter_push_char(c);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Handle typed characters while a task cell is being edited.
    ///
    /// Modelled on `handle_filter_field_input`, and tried before it: the cell
    /// editor owns every key the keymap did not claim, so an unbound letter
    /// types rather than falling through to a task-mode binding.
    fn handle_task_edit_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        use crossterm::event::{KeyCode, KeyModifiers};

        if !self.tasks.cell_edit_open() {
            return Ok(false);
        }
        // A value picker holds no text, so there is nothing for a character
        // to go into.
        if self.tasks.cell_edit_is_options() {
            return Ok(false);
        }

        match event.code {
            KeyCode::Backspace => {
                self.tasks.cell_edit_pop_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.tasks.cell_edit_push_char(c);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Move the caret in whichever filter editor is active.
    ///
    /// A date field being picked keeps its caret in the calendar, since that is
    /// what decides which end of a range the navigation keys rewrite; every other
    /// field keeps its own.
    fn move_filter_caret(&mut self, delta: i64) {
        if self.tasks.calendar_open() {
            self.tasks.filter_calendar_move_caret(delta);
        } else if self.tasks.cell_edit_open() {
            self.tasks.cell_edit_move_caret(delta);
        } else {
            self.tasks.filter_move_caret(delta);
        }
    }

    /// Runs one of the shared text motions on whichever buffer is being typed
    /// into.
    ///
    /// One implementation for both panes, because there is one buffer type
    /// behind them.
    fn move_text_caret(&mut self, motion: TextMotion) {
        if self.tasks.cell_edit_open() {
            match motion {
                TextMotion::WordBack => self.tasks.cell_edit_move_word(-1),
                TextMotion::WordForward => self.tasks.cell_edit_move_word(1),
                TextMotion::Start => self.tasks.cell_edit_jump_start(),
                TextMotion::End => self.tasks.cell_edit_jump_end(),
            }
            return;
        }
        match motion {
            TextMotion::WordBack => self.tasks.filter_move_word(-1),
            TextMotion::WordForward => self.tasks.filter_move_word(1),
            TextMotion::Start => self.tasks.filter_caret_to_start(),
            TextMotion::End => self.tasks.filter_caret_to_end(),
        }
    }

    /// Runs one of the shared cuts on whichever buffer is being typed into,
    /// and answers the command that puts what came out on the clipboard.
    ///
    /// Split the same way [`App::move_text_caret`] is, because the same two
    /// panes are behind it. A cut that took nothing answers `None`: `ctrl-d`
    /// at the end of a line should leave the clipboard holding whatever it
    /// already held rather than emptying it.
    fn cut_text(&mut self, cut: TextCut) -> Option<AppCommand> {
        let taken = match self.tasks.cell_edit_open() {
            true => self.tasks.cell_edit_cut(cut),
            false => self.tasks.filter_cut(cut),
        };
        (!taken.is_empty()).then_some(AppCommand::CopyToClipboard(taken))
    }

    /// Handle typed characters while the date picker is open.
    ///
    /// Typed text goes into the filter field the panel is showing, not into a
    /// buffer hidden in the overlay, and the table refilters as it lands. Only
    /// keys the keymap did not claim reach here, so the navigation letters stay
    /// navigation.
    fn handle_calendar_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        if !self.tasks.calendar_open() {
            return Ok(false);
        }

        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Backspace => {
                self.tasks.filter_calendar_pop_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                // With the grid up only the characters of a written date get
                // through. Every letter there either steers the grid or is one
                // stroke of a name whose other strokes do, so an `a` landing in
                // the text could only ever be half a keyword — and the half
                // that did not land moved the date out from under it. The key
                // to the names is drawn once the grid is put away, which is
                // also when they can be typed in full.
                //
                // Swallowed rather than passed on, so a rejected letter cannot
                // fall through to the task-cell editor underneath.
                if self.tasks.calendar_grid_visible()
                    && !crate::domain::is_date_char(c)
                {
                    return Ok(true);
                }
                self.tasks.filter_calendar_push_char(c);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Handle the sidebar's one-line prompt: typing a name for `w`, or
    /// answering the `y`/`n` confirmation for `d`.
    ///
    /// Read outside the keymap, like project search, so an unbound letter
    /// types instead of firing `b` or `d`. One mode covers both prompts: the
    /// handler already has the prompt in hand to know which it is.
    fn handle_filter_set_name_input(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<bool> {
        use crate::app::task::SidebarPrompt;
        use crossterm::event::{KeyCode, KeyModifiers};

        let Some(prompt) = self.tasks.filter_set_prompt().cloned() else {
            // The mode outlived its prompt, which nothing should do; leaving
            // it would swallow every key from here on.
            self.leave_sidebar_prompt();
            return Ok(false);
        };

        let typed = !event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match prompt {
            SidebarPrompt::Save { .. } => match event.code {
                KeyCode::Enter => {
                    self.commit_filter_set_save();
                    Ok(true)
                }
                KeyCode::Esc => {
                    self.tasks.filter_set_prompt_cancel();
                    self.leave_sidebar_prompt();
                    Ok(true)
                }
                KeyCode::Backspace => {
                    self.tasks.filter_set_prompt_pop_char();
                    Ok(true)
                }
                KeyCode::Left => {
                    self.tasks.filter_set_prompt_move_caret(-1);
                    Ok(true)
                }
                KeyCode::Right => {
                    self.tasks.filter_set_prompt_move_caret(1);
                    Ok(true)
                }
                KeyCode::Char(c) if typed => {
                    self.tasks.filter_set_prompt_push_char(c);
                    Ok(true)
                }
                _ => Ok(false),
            },
            SidebarPrompt::ConfirmDelete { .. } | SidebarPrompt::ConfirmLoad { .. } => {
                match event.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') if typed => {
                        match prompt {
                            SidebarPrompt::ConfirmLoad { name } => {
                                self.commit_filter_set_load(&name)
                            }
                            _ => self.commit_filter_set_delete(),
                        }
                        Ok(true)
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') if typed => {
                        self.tasks.filter_set_prompt_cancel();
                        self.leave_sidebar_prompt();
                        Ok(true)
                    }
                    KeyCode::Esc => {
                        self.tasks.filter_set_prompt_cancel();
                        self.leave_sidebar_prompt();
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    /// Opens the sidebar prompt, remembering the view it interrupted.
    ///
    /// The sets keys are the same keys in the filter view and the project
    /// view, so where a prompt returns to cannot be a constant: `w` pressed
    /// over the project list has to leave the project list on screen.
    fn enter_sidebar_prompt(&mut self) {
        self.prompt_return_mode = match self.mode {
            Mode::Project | Mode::ProjectSearch => Mode::Project,
            _ => Mode::Filter,
        };
        self.mode = Mode::FilterSetName;
    }

    /// Puts the keys back in the view the prompt interrupted.
    fn leave_sidebar_prompt(&mut self) {
        match self.prompt_return_mode {
            Mode::Project => self.set_project_mode(),
            _ => self.set_filter_mode(),
        }
    }

    /// Commits the open cell editor, or says why it cannot be committed.
    ///
    /// A refusal leaves the editor open: the value that could not be resolved
    /// is still on screen, and still the one to fix.
    fn commit_open_cell_edit(&mut self) {
        let context = self.edit_context();
        // Read before the commit closes the editor, because that is what
        // names a custom field in the confirmation: `TaskFieldEdit` carries
        // the gid, and the label lives on the table.
        let column = self
            .tasks
            .table()
            .columns
            .get(self.tasks.selected_column())
            .cloned();

        match self.tasks.commit_cell_edit(&context) {
            Ok(edits) => {
                // All three lists as one run of writes. They go to three
                // endpoints, but they are one edit as far as the user is
                // concerned — and so one question, with one count.
                let writes = edits
                    .fields
                    .into_iter()
                    .map(PendingEdit::Field)
                    .chain(edits.projects.into_iter().map(PendingEdit::Project))
                    .chain(edits.parents.into_iter().map(PendingEdit::Parent))
                    .collect::<Vec<_>>();
                // Before the gate: whichever way the question is answered,
                // the cell editor is closed and the keys belong to the table.
                // `submit_writes` moves on to `Mode::Confirm` from here when
                // it has to, and `return_mode` carries this back.
                self.set_task_mode();
                self.submit_writes(writes, column.as_deref());
            }
            Err(message) => self.tasks.set_edit_notice(message),
        }
    }

    /// Commits `w`: saves the panel under the typed name and binds to it.
    ///
    /// An empty name is refused with the prompt left open — an entry nothing
    /// can name is an entry nothing can load or delete.
    fn commit_filter_set_save(&mut self) {
        let name = self
            .tasks
            .filter_set_prompt_text()
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            self.tasks.set_filter_sets_notice("a name is required");
            return;
        }

        let entry = NamedFilterSet {
            name: name.clone(),
            scratch: false,
            // Always written, even as `[]`: an entry saved from here on is
            // unambiguous about the selection, and only a hand-written or
            // pre-existing one is silent.
            projects: Some(self.projects.selection_for_filter_set()),
            sets: self.tasks.filter_sets_to_saved(),
        };
        match self
            .config
            .filter_sets
            .iter_mut()
            .find(|existing| !existing.scratch && existing.name.eq_ignore_ascii_case(&name))
        {
            Some(existing) => *existing = entry,
            None => self.config.filter_sets.push(entry),
        }

        self.tasks.filter_set_prompt_cancel();
        self.tasks.filter_set_bind(name);
        self.leave_sidebar_prompt();
        self.write_config_or_report();
    }

    /// Commits `d`: removes the loaded entry and detaches the panel.
    fn commit_filter_set_delete(&mut self) {
        if let Some(name) = self.tasks.filter_set_loaded_name().map(str::to_string) {
            self.config
                .filter_sets
                .retain(|entry| entry.scratch || !entry.name.eq_ignore_ascii_case(&name));
        }

        self.tasks.filter_set_prompt_cancel();
        self.tasks.filter_set_detach();
        self.clamp_filter_sets_page();
        self.leave_sidebar_prompt();
        self.write_config_or_report();
    }

    /// The saved entry a digit addresses, if there is one there.
    fn filter_set_at(&self, position: u8) -> Option<NamedFilterSet> {
        let index = self
            .tasks
            .filter_sets_page_start()
            .saturating_add(position.saturating_sub(1) as usize);
        self.config
            .sorted_filter_sets()
            .get(index)
            .map(|entry| (*entry).clone())
    }

    /// Replaces the panel with a saved entry, by name.
    ///
    /// By name rather than by position because a confirmation can sit between
    /// the digit and the load, and a window that moved in between must not
    /// load a different entry than the one the prompt named.
    fn load_named_filter_set(&mut self, name: &str) {
        // Anything the bound panel was still holding goes to disk before it
        // is replaced. The write-through normally runs *after* the key, by
        // which point this key has already thrown the panel away.
        self.sync_named_filter_set();

        let Some(entry) = self
            .config
            .filter_sets
            .iter()
            .find(|entry| !entry.scratch && entry.name == name)
            .cloned()
        else {
            return;
        };

        self.tasks.filter_sets_load(&entry.name, &entry.sets);
        // Always, and with no "leave it alone" case: the selection is part of
        // what the entry *is*, so an entry that names no projects — a set
        // migrated from an older file, or one hand-written without the key —
        // loads to nothing selected and parks you in the project view.
        self.projects.apply_filter_set_selection(entry.projects());
    }

    /// The selection the session starts from.
    ///
    /// One place, two slots: the entry `[view].filter_set` names, or — when
    /// it names none — the scratch entry an unnamed panel left behind. There
    /// is no third answer, because there is nowhere else a selection is
    /// recorded.
    ///
    /// A bound name that matches no entry restores nothing rather than
    /// falling back to the scratch slot: that selection belongs to a
    /// different panel, and inheriting it would silently ask a saved
    /// question of projects the saved question never named.
    fn restored_selection(&self) -> Vec<String> {
        match self.config.view.filter_set.as_deref() {
            Some(name) => self
                .config
                .named_filter_set(name)
                .map(|entry| entry.projects().to_vec())
                .unwrap_or_default(),
            None => self.config.scratch_projects().to_vec(),
        }
    }

    /// Commits the `y` at a load confirmation: the unnamed panel goes.
    fn commit_filter_set_load(&mut self, name: &str) {
        self.tasks.filter_set_prompt_cancel();
        self.leave_sidebar_prompt();

        let task_targets_before = self.task_targets_before();
        self.load_named_filter_set(name);
        self.enforce_project_gate();
        self.update_task_data_after_action(task_targets_before);
    }

    /// Keeps the numbered window pointing at entries that still exist.
    fn clamp_filter_sets_page(&mut self) {
        let total = self.config.named_filter_set_count();
        self.tasks.filter_sets_page(0, total);
    }

    /// Writes the config, reporting a failure on the sidebar rather than
    /// returning it.
    ///
    /// Project visibility can afford to propagate a write error because it
    /// happens on one deliberate keypress; this also runs while someone is
    /// typing into a filter, and an unwritable config must not take the
    /// session down mid-word.
    fn write_config_or_report(&mut self) {
        // The load-bearing half of §3.2: `stage_view_state` and the
        // filter-set write-through both fire on a settled burst of
        // keystrokes, so without this the first `j` would rewrite the file in
        // version 2 behind the prompt — and a user who meant to quit and edit
        // it by hand would find it already changed.
        if self.pending_migration.is_some() {
            return;
        }
        match self.config.save_to_source_path() {
            Ok(()) => self.tasks.clear_filter_sets_notice(),
            Err(err) => {
                debug_log(&format!("filter set write failed: {err}"));
                self.tasks
                    .set_filter_sets_notice(format!("could not save: {err}"));
            }
        }
    }

    /// Writes the panel back to the named entry it was loaded from.
    ///
    /// No-op unless the panel is bound and something actually changed. The
    /// comparison is what makes the deliberately over-eager `dirty` flag safe.
    fn sync_named_filter_set(&mut self) {
        if self.stage_named_filter_set() {
            self.write_config_or_report();
        }
    }

    /// Updates the in-memory config from the panel, without writing.
    ///
    /// Split from the write so that a key which changes both the bound entry
    /// and the view — a digit that loads a set is both — costs one write
    /// rather than two.
    fn stage_named_filter_set(&mut self) -> bool {
        let fields_dirty = self.tasks.filter_set_dirty();
        // Cleared whether or not anything is written, so a config that cannot
        // be written reports once rather than once per keystroke.
        self.tasks.clear_filter_set_dirty();

        let name = self.tasks.filter_set_loaded_name().map(str::to_string);

        // The projects half runs independently of the dirty flag: the project
        // list has no such flag, and the comparison against the baseline is
        // cheap — two short sorted `Vec<String>`s — and is already the safety
        // net that makes the over-eager flag on the fields half safe.
        let selection = self.projects.selection_for_filter_set();
        let selection_moved = match &self.bound_projects {
            Some((bound, baseline)) if bound.as_deref().map(str::to_lowercase)
                == name.as_deref().map(str::to_lowercase) =>
            {
                *baseline != selection
            }
            // A binding that has just changed has not been edited through:
            // whatever the load landed on becomes the baseline, and nothing
            // is written until the user moves it.
            _ => false,
        };
        self.bound_projects = Some((name.clone(), selection.clone()));

        if !fields_dirty && !selection_moved {
            return false;
        }

        // An unnamed panel has a selection too, and since version 3 the only
        // place one can live is a `[[filter_set]]` — so it goes to the
        // scratch entry. Its fields deliberately do not: the panel is still
        // unnamed, they still exist nowhere but on screen, and the discard
        // confirmation still means what it says.
        let Some(name) = name else {
            if !selection_moved || self.config.scratch_projects() == selection.as_slice() {
                return false;
            }
            self.config.set_scratch_projects(selection);
            return true;
        };

        let sets = self.tasks.filter_sets_to_saved();
        let Some(entry) = self
            .config
            .filter_sets
            .iter_mut()
            .find(|entry| !entry.scratch && entry.name.eq_ignore_ascii_case(&name))
        else {
            return false;
        };

        let mut changed = false;
        if fields_dirty && entry.sets != sets {
            entry.sets = sets;
            changed = true;
        }
        // An entry migrated from an older file gains the key the first time
        // the selection moves while it is loaded.
        if selection_moved && entry.projects.as_deref() != Some(selection.as_slice()) {
            entry.projects = Some(selection);
            changed = true;
        }

        changed
    }

    /// The panes as they stand, in the shape `[view]` keeps.
    ///
    /// No selection: since version 3 that lives on a `[[filter_set]]`, and
    /// `legacy_projects` is read-only wreckage of the version the key was
    /// written in.
    fn current_view_config(&self) -> ViewConfig {
        ViewConfig {
            filter_set: self.tasks.filter_set_loaded_name().map(str::to_string),
            top_pane: if self.panel_size.is_minimized() {
                TopPaneState::Minimized
            } else if self.panel_size.is_maximized() {
                TopPaneState::Maximized
            } else {
                TopPaneState::Normal
            },
            tasks: self.tasks.visible(),
            filters: self.tasks.filter_panel_visible(),
            filter_sidebar: self.tasks.filter_sets_sidebar_visible(),
            recent: self.tasks.recent_pane_enabled(),
            legacy_projects: Vec::new(),
        }
    }

    /// Updates the in-memory `[view]`, answering whether anything moved.
    fn stage_view_state(&mut self) -> bool {
        let view = self.current_view_config();
        if self.config.view == view {
            return false;
        }
        self.config.view = view;
        true
    }

    fn handle_project_search_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.projects.end_search();
                self.set_project_mode();
                Ok(true)
            }
            KeyCode::Backspace => {
                self.projects.pop_search_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.projects.push_search_char(c.to_ascii_lowercase());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The task targets as they stand, for `update_task_data_after_action` to
    /// compare against once an action has run.
    fn task_targets_before(&self) -> Option<Vec<String>> {
        match self.tasks.status() {
            crate::app::task::TaskStatus::Idle => None,
            _ => Some(self.task_target_project_ids()),
        }
    }

    pub fn handle_action(
        &mut self,
        action: &Action,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        if let Some(command) = action.as_app_command() {
            debug_log(&format!("app command: {command:?}"));
            return Ok(Some(command));
        }

        let task_targets_before = self.task_targets_before();

        let mode = self.mode;
        debug_log(&format!(
            "handle_action: action={action} mode={:?} visible_tasks={}",
            mode,
            self.tasks.visible(),
        ));

        match action {
            Action::BeginFilterEdit => {
                // A date field is edited on the calendar rather than as raw
                // text, so it gets its own mode instead of the text buffer.
                if self.tasks.filter_calendar_begin() {
                    self.set_calendar_mode();
                } else {
                    self.begin_filter_field_edit();
                    self.set_filter_edit_mode();
                }
                return Ok(None);
            }
            Action::ToggleRecentPane => {
                self.tasks.toggle_recent_pane();
                return Ok(None);
            }
            Action::CompleteCandidate(delta) => {
                self.complete_candidate(*delta);
                return Ok(None);
            }
            // `ctrl-n` has meant "negate this filter row" since milestone 13,
            // and with a candidate overlay up it is also the obvious key for
            // "next candidate". Gated on the overlay rather than moved, in
            // the shape `calendar_grid_visible` already uses: with the
            // overlay open it walks, with none it negates as it always did.
            // `!` on the row in browse mode is the other way to negate.
            Action::FilterNegateField if self.tasks.completion_overlay_open() => {
                self.highlight_candidate(1);
                return Ok(None);
            }
            Action::CalendarCommit => {
                // A task date picked on the calendar commits the cell, not a
                // filter: the same key, one layer over.
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.calendar_normalize();
                    self.commit_open_cell_edit();
                    return Ok(None);
                }
                self.tasks.filter_calendar_commit();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::CalendarClose => {
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.cancel_cell_edit();
                    self.set_task_mode();
                    return Ok(None);
                }
                self.tasks.filter_calendar_close();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::FilterCaretLeft => {
                self.move_filter_caret(-1);
                return Ok(None);
            }
            Action::FilterCaretRight => {
                self.move_filter_caret(1);
                return Ok(None);
            }
            Action::CalendarJumpToStart => {
                self.tasks.filter_calendar_jump_to_start();
                return Ok(None);
            }
            Action::CalendarJumpToEnd => {
                self.tasks.filter_calendar_jump_to_end();
                return Ok(None);
            }
            Action::CalendarClear => {
                // On a task date `d` is the whole gesture: it empties the
                // value and sends it. Leaving the picker open would strand
                // the user, since `enter` normalizes an empty query back into
                // the highlighted day and would quietly re-set the date they
                // just cleared.
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.calendar_clear_text();
                    self.commit_open_cell_edit();
                    return Ok(None);
                }
                self.tasks.filter_calendar_clear();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::CalendarPrevDay => {
                self.tasks.filter_calendar_move_days(-1);
                return Ok(None);
            }
            Action::CalendarNextDay => {
                self.tasks.filter_calendar_move_days(1);
                return Ok(None);
            }
            Action::CalendarPrevMonth => {
                self.tasks.filter_calendar_move_months(-1);
                return Ok(None);
            }
            Action::CalendarNextMonth => {
                self.tasks.filter_calendar_move_months(1);
                return Ok(None);
            }
            Action::CalendarToday => {
                self.tasks.filter_calendar_today();
                return Ok(None);
            }
            Action::CalendarToggleGrid => {
                self.tasks.toggle_calendar_grid();
                return Ok(None);
            }
            Action::FilterDoneEditing => {
                self.finish_filter_completion();
                self.tasks.filter_calendar_close();
                self.tasks.filter_edit_done();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::FilterCancelEditing => {
                self.finish_filter_completion();
                self.tasks.filter_calendar_close();
                self.tasks.filter_edit_done();
                self.tasks.toggle_filter_panel();
                self.set_task_mode();
                return Ok(None);
            }
            Action::FilterSetsToggle => {
                self.tasks.filter_sets_toggle_sidebar();
                self.clamp_filter_sets_page();
                return Ok(None);
            }
            Action::FilterSetsPageBack => {
                self.tasks
                    .filter_sets_page(-1, self.config.named_filter_set_count());
                return Ok(None);
            }
            Action::FilterSetsPageForward => {
                self.tasks
                    .filter_sets_page(1, self.config.named_filter_set_count());
                return Ok(None);
            }
            Action::FilterSetLoad(position) => {
                let Some(entry) = self.filter_set_at(*position) else {
                    return Ok(None);
                };
                // A bound panel is already on disk, so loading over it costs
                // nothing. An unnamed one that is filtering exists nowhere
                // else, and the digit did not ask for it to be thrown away.
                if self.tasks.filter_set_is_unsaved() {
                    self.tasks.filter_set_prompt_confirm_load(&entry.name);
                    self.enter_sidebar_prompt();
                    return Ok(None);
                }

                self.load_named_filter_set(&entry.name);
                // An entry that names no projects — one migrated from an
                // older file — leaves nothing selected.
                self.enforce_project_gate();
                // Deliberately not an early return past the fetch decision:
                // loading an entry can widen the due window pushed down to
                // Asana, and `TaskQuery::covers` records a fetched window as
                // cached — so skipping this leaves rows permanently missing
                // rather than merely late.
                self.update_task_data_after_action(task_targets_before);
                return Ok(None);
            }
            Action::FilterSetSave => {
                self.tasks.filter_set_prompt_save();
                self.enter_sidebar_prompt();
                return Ok(None);
            }
            Action::FilterSetDelete => {
                if self.tasks.filter_set_prompt_delete() {
                    self.enter_sidebar_prompt();
                }
                return Ok(None);
            }
            Action::FilterSetCopyToNew => {
                self.tasks.filter_set_detach();
                return Ok(None);
            }
            Action::FilterSetNew => {
                // Same reason as a load: the panel is about to go, and the
                // write-through does not run until after this key.
                self.sync_named_filter_set();
                self.tasks.filter_set_new();
                // The projects are part of what there is to start from now,
                // so leaving them behind would make `n` a half-measure. The
                // cost — an empty table, and a reselection before anything
                // loads — is what `u` is for: this pushes the selection
                // history for the same reason a load does.
                self.projects.clear_selection();
                // `n` is pressed in the filter view as often as in the
                // project view, and it has just made the filter view
                // unreachable.
                self.enforce_project_gate();
                // Clearing a due filter widens the window pushed down to
                // Asana, for the same reason loading an entry can, so this
                // takes the same route past the fetch decision.
                self.update_task_data_after_action(task_targets_before);
                return Ok(None);
            }
            Action::SetProjectMode => {
                self.set_project_mode();
                return Ok(None);
            }
            Action::SetFilterMode => {
                self.set_filter_mode();
                return Ok(None);
            }
            Action::SetTaskMode => {
                self.set_task_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::SetGanttMode => {
                self.set_gantt_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::ToggleGantt => {
                self.tasks.gantt_mut().toggle_visible();
                // Turning the chart off leaves nothing for gantt mode to
                // drive, so the mode goes with it.
                if !self.tasks.gantt().visible() {
                    self.set_task_mode();
                }
                return Ok(None);
            }
            Action::GanttAddColumn => {
                self.tasks.gantt_add_column();
                return Ok(None);
            }
            Action::GanttRemoveColumn => {
                self.tasks.gantt_remove_column();
                return Ok(None);
            }
            Action::CycleGanttColorKey => {
                self.tasks.cycle_gantt_color_key();
                return Ok(None);
            }
            // Scrolling and zooming move a viewport over the rows already in
            // the table. None of these issues a request or changes a filter.
            Action::GanttScrollLeft => {
                self.tasks.gantt_scroll(false);
                return Ok(None);
            }
            Action::GanttScrollRight => {
                self.tasks.gantt_scroll(true);
                return Ok(None);
            }
            Action::GanttZoomIn => {
                self.tasks.gantt_zoom(true);
                return Ok(None);
            }
            Action::GanttZoomOut => {
                self.tasks.gantt_zoom(false);
                return Ok(None);
            }
            Action::GanttZoomFit => {
                self.tasks.gantt_fit();
                return Ok(None);
            }
            Action::GanttToday => {
                self.tasks.gantt_today();
                return Ok(None);
            }
            Action::GanttOpenOrder => {
                self.tasks.gantt_open_order();
                self.mode = Mode::GanttOrder;
                return Ok(None);
            }
            Action::GanttOrderMoveUp => {
                self.tasks.gantt_order_move(MoveTo::Up);
                return Ok(None);
            }
            Action::GanttOrderMoveDown => {
                self.tasks.gantt_order_move(MoveTo::Down);
                return Ok(None);
            }
            Action::GanttOrderMoveTop => {
                self.tasks.gantt_order_move(MoveTo::Top);
                return Ok(None);
            }
            Action::GanttOrderMoveBottom => {
                self.tasks.gantt_order_move(MoveTo::Bottom);
                return Ok(None);
            }
            Action::GanttOrderCommit => {
                self.tasks.gantt_mut().dialog_commit();
                self.mode = Mode::Gantt;
                self.persist_gantt_colors()?;
                return Ok(None);
            }
            Action::GanttOrderCancel => {
                self.tasks.gantt_mut().dialog_cancel();
                self.mode = Mode::Gantt;
                return Ok(None);
            }
            Action::EditColumn => {
                let context = self.edit_context();
                match self.tasks.begin_cell_edit(&context) {
                    Ok(()) => self.set_task_edit_mode(),
                    Err(message) => self.tasks.set_edit_notice(message),
                }
                return Ok(None);
            }
            Action::CancelColumnEdit => {
                // A draft is thrown away whole rather than left as an empty
                // row: nothing was sent, so there is nothing to keep.
                match self.tasks.draft_gid().is_some() {
                    true => {
                        self.tasks.cancel_draft();
                        self.set_edit_mode();
                    }
                    false => {
                        self.tasks.cancel_cell_edit();
                        self.set_task_mode();
                    }
                }
                return Ok(None);
            }
            Action::SetEditMode => {
                self.set_edit_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::EditCancel => {
                self.edit_cancel();
                return Ok(None);
            }
            Action::InsertTask => {
                self.insert_task(false);
                return Ok(None);
            }
            Action::InsertSubtask => {
                self.insert_task(true);
                return Ok(None);
            }
            Action::InsertSection => {
                self.insert_section();
                return Ok(None);
            }
            Action::DeleteSection => {
                self.delete_section();
                return Ok(None);
            }
            Action::MoveTaskToSection(delta) => {
                self.move_task_to_section(*delta);
                return Ok(None);
            }
            Action::MarkForDeletion => {
                self.tasks.toggle_deletion_marks();
                return Ok(None);
            }
            Action::DeleteMarkedTasks => {
                self.delete_marked_tasks();
                return Ok(None);
            }
            Action::HighlightCandidate(delta) => {
                self.highlight_candidate(*delta);
                return Ok(None);
            }
            Action::ColumnEditCycleValue(delta) => {
                self.tasks.cell_edit_cycle_value(*delta);
                return Ok(None);
            }
            Action::ColumnEditClear => {
                self.tasks.cell_edit_clear();
                return Ok(None);
            }
            Action::TextCaretWordBack => {
                self.move_text_caret(TextMotion::WordBack);
                return Ok(None);
            }
            Action::TextCaretWordForward => {
                self.move_text_caret(TextMotion::WordForward);
                return Ok(None);
            }
            Action::TextCaretStart => {
                self.move_text_caret(TextMotion::Start);
                return Ok(None);
            }
            Action::TextCaretEnd => {
                self.move_text_caret(TextMotion::End);
                return Ok(None);
            }
            // Not `Ok(None)`: the cut hands back what it took, and the
            // clipboard is the runtime's to write.
            Action::TextCutChar => {
                return Ok(self.cut_text(TextCut::Char));
            }
            Action::TextCutWord => {
                return Ok(self.cut_text(TextCut::Word));
            }
            Action::TextCutToEnd => {
                return Ok(self.cut_text(TextCut::ToEnd));
            }
            // Deliberately not early returns: both send a write, and the tail
            // is what keeps the fetch decision running after an action, for
            // the same reason `FilterSetLoad` falls through to it.
            Action::CommitColumnEdit if self.tasks.draft_gid().is_some() => {
                self.commit_draft();
                return Ok(None);
            }
            Action::CommitColumnEdit => {
                self.commit_open_cell_edit();
            }
            Action::ToggleTaskCompleted if self.tasks.visible() => {
                let edits = self.tasks.toggle_completed_edits();
                self.submit_writes(edits.into_iter().map(PendingEdit::Field).collect(), None);
            }
            // The dialog is a list of its own, so the cursor keys drive it
            // rather than the task rows underneath. Same shape as the filter
            // panel's interception of the same two actions.
            Action::MoveUp if self.mode == Mode::GanttOrder => {
                self.tasks.gantt_mut().dialog_move_cursor(-1);
                return Ok(None);
            }
            Action::MoveDown if self.mode == Mode::GanttOrder => {
                self.tasks.gantt_mut().dialog_move_cursor(1);
                return Ok(None);
            }
            Action::ToggleTaskMode => {
                if self.mode == Mode::Task {
                    self.set_project_mode();
                } else {
                    self.set_task_mode();
                    self.ensure_task_data();
                }
                return Ok(None);
            }
            Action::ResizeTopPaneUp => {
                self.adjust_top_pane_height(2);
                return Ok(None);
            }
            Action::ResizeTopPaneDown => {
                self.adjust_top_pane_height(-2);
                return Ok(None);
            }
            Action::MinimizeTopPane => {
                if self.panel_size.is_minimized() {
                    self.restore_top_pane();
                } else {
                    self.minimize_top_pane();
                }
                self.set_task_mode();
                return Ok(None);
            }
            Action::MaximizeTopPane => {
                if self.panel_size.is_maximized() {
                    self.restore_top_pane();
                } else {
                    self.maximize_top_pane();
                }
                return Ok(None);
            }
            Action::RestoreTopPane => {
                self.restore_top_pane();
                return Ok(None);
            }
            Action::Open => {
                let url = if matches!(mode, Mode::Task) {
                    self.tasks.selected_task_url()
                } else if matches!(mode, Mode::Project | Mode::ProjectSearch) {
                    self.projects.selected_project().and_then(|p| match p.kind {
                        ProjectKind::Normal => Some(format!("https://app.asana.com/0/{}", p.id)),
                        ProjectKind::AssignedToMe => None,
                    })
                } else {
                    None
                };
                if let Some(url) = url {
                    return Ok(Some(AppCommand::OpenUrl(url)));
                }
                return Ok(None);
            }
            Action::ToggleTaskFilters => {
                if self.tasks.filter_panel_visible() {
                    self.tasks.toggle_filter_panel();
                    self.set_task_mode();
                } else {
                    self.set_filter_mode();
                }
                return Ok(None);
            }
            Action::ScrollLeft => {
                if self.tasks.visible() {
                    self.tasks.scroll_left();
                    return Ok(None);
                }
            }
            Action::ScrollRight => {
                if self.tasks.visible() {
                    self.tasks.scroll_right();
                    return Ok(None);
                }
            }
            Action::ToggleCompletedFilter if self.tasks.visible() => {
                let targets = self.task_target_projects();
                self.tasks.cycle_completed_filter_without_refresh();
                let query_template = self.tasks.desired_task_query();
                if self.tasks.can_serve_query_for_targets(&targets, &query_template) {
                    if self.task_data_receiver.is_some() {
                        self.task_data_generation = self.task_data_generation.wrapping_add(1);
                        self.task_data_receiver = None;
                    }
                    self.tasks.refresh_from_cache();
                } else {
                    self.start_task_data_fetch();
                }
                return Ok(None);
            }
            _ => {}
        }

        let task_action = action.is_task_view_action() && self.tasks.visible();

        let result = if matches!(mode, Mode::Project | Mode::ProjectSearch) && !task_action
        {
            self.projects.apply_action(action, page_size)
        } else {
            let filter_focused = self.filter_panel_focused();
            self.tasks.apply_action(action, page_size, filter_focused)
        };

        if matches!(action, Action::StartSearch) && matches!(mode, Mode::Project) {
            self.set_project_search_mode();
        } else if matches!(action, Action::ClearSearch)
            && matches!(mode, Mode::ProjectSearch)
        {
            self.set_project_mode();
        }

        if matches!(
            action,
            Action::ToggleStarredSelected | Action::ToggleHiddenSelected
        ) {
            self.persist_project_visibility()?;
        }

        self.update_task_data_after_action(task_targets_before);

        Ok(result)
    }

    pub fn handle_key_event(
        &mut self,
        keymap: &KeyMap,
        event: crossterm::event::KeyEvent,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        let result = self.handle_key_event_inner(keymap, event, page_size);
        // The last key of a burst is the one with an empty queue behind it,
        // which is the same moment `settle_table` picks to rebuild. Typing a
        // filter is one write, not one per character.
        //
        // Here rather than in `handle_action` because the prompt's keys, and
        // the filter-field ones, are read outside the keymap entirely — and
        // because the view has the same problem from the other end: a pane
        // can be opened from half a dozen actions and from none of them.
        //
        // Both are staged before either is written, so a key that moves both
        // — a digit that loads a named set moves the panel and the binding —
        // costs one write.
        if !self.tasks.input_pending() {
            let filter_set_changed = self.stage_named_filter_set();
            let view_changed = self.stage_view_state();
            if filter_set_changed || view_changed {
                self.write_config_or_report();
            }
        }
        result
    }

    fn handle_key_event_inner(
        &mut self,
        keymap: &KeyMap,
        event: crossterm::event::KeyEvent,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        if matches!(
            KeyBinding::from_crossterm_event(event),
            Some(KeyBinding::Ctrl('c'))
        ) {
            return Ok(Some(AppCommand::Quit));
        }

        // The migration prompt owns every key it is shown, and nothing below
        // it may run: `ctrl-c` above is the one way out that is not one of
        // its own three keys.
        if self.pending_migration.is_some() {
            return Ok(self.handle_migration_input(event));
        }

        // `esc` dismisses the notice pane, in whatever mode it is showing
        // over. Here rather than as an action because `esc` is already bound
        // to something else in half the modes a notice can be raised in, and
        // the notice is the topmost thing on screen in all of them.
        //
        // It does not consume the key: the same press still cancels the edit
        // or closes the panel it always did. A notice that cost a keystroke
        // to clear would be worse than one that stayed.
        //
        // Ahead of the polls, so a write that fails on this very keystroke is
        // read rather than cleared unseen.
        if matches!(KeyBinding::from_crossterm_event(event), Some(KeyBinding::Esc)) {
            self.tasks.clear_edit_notice();
        }

        self.poll_task_data();
        self.poll_task_edits();

        debug_log(&format!(
            "key event: {:?} {:?}",
            event.code, event.modifiers
        ));

        // Under the migration prompt and over everything else, matching the
        // order the overlays are drawn in: a confirmation is a question about
        // a write that has not happened, and no key may do anything else
        // until it is answered.
        //
        // Every key, answered or not: a modal that let an unrecognized letter
        // through to the table underneath would be a modal in name only.
        if matches!(self.mode, Mode::Confirm) {
            self.handle_confirm_input(event);
            return Ok(None);
        }

        // Ahead of the keymap: the prompt owns every key it is shown, so an
        // unbound letter types rather than falling through to a binding.
        if matches!(self.mode, Mode::FilterSetName)
            && self.handle_filter_set_name_input(event)?
        {
            return Ok(None);
        }

        if let Some(binding) = KeyBinding::from_crossterm_event(event) {
            debug_log(&format!("resolved binding: {binding:?}"));
            if let Some(action) = keymap.action_for(&binding, self.mode).cloned() {
                debug_log(&format!("resolved action: {action}"));
                // `j`, `k`, and `d` drive a value picker and type into
                // anything else, the same way the filter panel's label keys
                // are context-sensitive. Only a plain character is ambiguous:
                // `ctrl-l` clears whatever the editor holds.
                if action.is_column_edit_value_action()
                    && matches!(self.mode, Mode::ColumnEdit)
                    && !self.tasks.cell_edit_is_options()
                    && !event
                        .modifiers
                        .intersects(crossterm::event::KeyModifiers::CONTROL
                            | crossterm::event::KeyModifiers::ALT)
                {
                    self.handle_task_edit_input(event)?;
                    return Ok(None);
                }
                // The mirror of that rule for the date picker: with the
                // month grid hidden its navigation letters are not navigation
                // at all, they are the letters of `tuesday`. A rebinding onto
                // a modifier keeps working, since only a plain character is
                // ambiguous.
                if action.is_calendar_grid_action()
                    && matches!(self.mode, Mode::Calendar)
                    && !self.tasks.calendar_grid_visible()
                    && !event
                        .modifiers
                        .intersects(crossterm::event::KeyModifiers::CONTROL
                            | crossterm::event::KeyModifiers::ALT)
                {
                    self.handle_calendar_input(event)?;
                    return Ok(None);
                }
                if action.is_label_filter_action() {
                    // Context-sensitive: navigate labels when a labels field is selected,
                    // otherwise fall back to pushing the character.
                    self.handle_filter_field_input(event, Some(&action))?;
                    return Ok(None);
                }
                return self.handle_action(&action, page_size);
            }
            debug_log("no action for binding");
        }

        if self.handle_calendar_input(event)? {
            return Ok(None);
        }

        if self.handle_task_edit_input(event)? {
            return Ok(None);
        }

        if self.filter_panel_focused() {
            if self.handle_filter_field_input(event, None)? {
                return Ok(None);
            }
        }

        if self.projects.search_active() {
            if self.handle_project_search_input(event)? {
                return Ok(None);
            }
        }

        Ok(None)
    }

    /// What resolving an edit needs that the task pane does not own.
    fn edit_context(&mut self) -> crate::app::task_edit::EditContext {
        crate::app::task_edit::EditContext {
            today: Some(crate::domain::today()),
            current_user_gid: self.current_user_gid.clone(),
            projects: self
                .projects
                .all_projects()
                .iter()
                .filter(|project| matches!(project.kind, ProjectKind::Normal))
                .map(|project| (project.id.clone(), project.name.clone()))
                .collect(),
            people: self.people_directory(),
            // Any project answers: every one of them carries its workspace,
            // and a task created outside every project still needs one.
            workspace_gid: self
                .config
                .auth
                .as_ref()
                .and_then(|auth| auth.workspace_gid.clone())
                .or_else(|| {
                    self.projects
                        .all_projects()
                        .iter()
                        .find_map(|project| project.workspace_gid.clone())
                }),
        }
    }

    /// The workspace directory, fetched at most once per session.
    ///
    /// Synchronous, unlike the task fetch: it happens when an editor opens
    /// rather than while the user is reading, it is one request, and an
    /// editor that opened without its candidates would be an editor that
    /// refuses every name typed into it.
    fn people_directory(&mut self) -> Vec<(String, String)> {
        if let Some(people) = &self.people {
            return people.clone();
        }

        let people = match self.client.list_users() {
            Ok(users) => users
                .into_iter()
                .filter_map(|user| Some((user.gid, user.name.or(user.display_name)?)))
                .collect(),
            Err(err) => {
                // Not fatal and not retried: the picker falls back to the
                // people the loaded tasks name, and a workspace that will not
                // answer will not answer on the next keystroke either.
                debug_log(&format!("user directory unavailable: {err}"));
                Vec::new()
            }
        };
        self.people = Some(people.clone());
        people
    }

    /// Sends a run of writes, or asks first if there are enough of them.
    ///
    /// The gate every bulk edit goes through. Below the threshold the writes
    /// go straight out, which is what keeps editing one row as immediate as
    /// it was. Above it nothing happens at all until the count on screen is
    /// agreed to — not even the optimistic local update, so cancelling leaves
    /// a table that never moved.
    ///
    /// `column` is the table's label for the column being written, which is
    /// the only way a custom field is named rather than numbered in the
    /// question.
    fn submit_writes(&mut self, writes: Vec<PendingEdit>, column: Option<&str>) {
        if writes.is_empty() {
            return;
        }

        // Counted in writes rather than in tasks. They are the same number
        // for a field edit, and for a projects edit one task can be several
        // writes — which is the case where the larger number is the honest
        // one, because it is the one the server will be asked to do.
        if !self.config.edit.needs_confirmation(writes.len()) {
            self.apply_and_enqueue(writes);
            return;
        }

        self.tasks.clear_edit_notice();
        self.pending_bulk_edit = Some(PendingBulkEdit {
            summary: bulk_edit_summary(&writes, column),
            writes,
            return_mode: self.mode,
        });
        self.mode = Mode::Confirm;
    }

    /// Applies writes locally and queues them. Past every confirmation.
    fn apply_and_enqueue(&mut self, writes: Vec<PendingEdit>) {
        let fields = writes
            .iter()
            .filter_map(|write| match write {
                PendingEdit::Field(edit) => Some(edit.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let projects = writes
            .iter()
            .filter_map(|write| match write {
                PendingEdit::Project(edit) => Some(edit.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let parents = writes
            .iter()
            .filter_map(|write| match write {
                PendingEdit::Parent(edit) => Some(edit.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        // One rebuild each rather than one for the lot: the three apply to
        // different parts of the record and each already batches its own.
        if !fields.is_empty() {
            self.tasks.apply_edits_locally(&fields);
        }
        if !projects.is_empty() {
            self.tasks.apply_project_edits_locally(&projects);
        }
        if !parents.is_empty() {
            self.tasks.apply_parent_edits_locally(&parents);
        }

        self.enqueue_writes(writes);
    }

    /// The bulk edit waiting on an answer, for the renderer.
    pub fn pending_bulk_edit_view(&self) -> Option<(usize, String)> {
        self.pending_bulk_edit
            .as_ref()
            .map(|pending| (pending.count(), pending.summary.clone()))
    }

    /// `y`: sends the bulk edit that was waiting.
    fn confirm_bulk_edit(&mut self) {
        let Some(pending) = self.pending_bulk_edit.take() else {
            return;
        };
        self.mode = pending.return_mode;
        self.apply_and_enqueue(pending.writes);
    }

    /// `n` or `esc`: drops it. Nothing was applied, so nothing is undone.
    ///
    /// The count is said back deliberately: the one thing worse than an
    /// unnoticed bulk edit is an unnoticed cancelled one, and the cell the
    /// user typed into has already gone back to its old value on screen.
    fn cancel_bulk_edit(&mut self) {
        let Some(pending) = self.pending_bulk_edit.take() else {
            return;
        };
        self.mode = pending.return_mode;
        self.tasks.set_edit_notice(format!(
            "cancelled: {} tasks unchanged",
            pending.count()
        ));
    }

    /// Reads the one key a confirmation accepts, outside the keymap.
    ///
    /// Outside it for the same reason the sidebar's prompt is: every other
    /// key has to be swallowed rather than fall through to the binding it
    /// carries, and `j` reaching the table under a modal would move a cursor
    /// nobody can see.
    fn handle_confirm_input(&mut self, event: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let typed = !event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if typed => self.confirm_bulk_edit(),
            KeyCode::Char('n') | KeyCode::Char('N') if typed => self.cancel_bulk_edit(),
            KeyCode::Esc => self.cancel_bulk_edit(),
            // Everything else is ignored rather than acted on. The caller
            // swallows it either way.
            _ => {}
        }
    }

    /// Chunks writes to the batch limit and queues them for the pool.
    ///
    /// The one place every background write goes out, whichever endpoint it
    /// is bound for: a batch can carry a field change, a membership change,
    /// and a re-parenting together, and they are counted and reported as one
    /// burst regardless.
    fn enqueue_writes(&mut self, writes: Vec<PendingEdit>) {
        if writes.is_empty() {
            return;
        }

        self.tasks.clear_edit_notice();
        self.begin_writes(writes.len());

        let pool = self
            .write_pool
            .get_or_insert_with(|| WritePool::new(&self.client, &self.task_edit_events.0));
        for chunk in writes.chunks(MAX_BATCH_ACTIONS) {
            pool.submit(chunk.to_vec());
        }
    }

    fn begin_writes(&mut self, count: usize) {
        self.task_edits_sent += count;
        self.task_edits_outstanding += count;
    }

    /// Reconciles whatever writes have come back.
    ///
    /// A success takes the server's `modified_at`; a failure puts the field
    /// back the way it was. Both are keyed by gid, so replies arriving out of
    /// order — or belonging to different batches — need no bookkeeping.
    pub fn poll_task_edits(&mut self) {
        loop {
            match self.task_edit_events.1.try_recv() {
                Ok(message) => self.reconcile_task_edit(message),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Waits for every in-flight write to be reconciled.
    ///
    /// The event loop never calls this; it polls. Tests and a shutdown that
    /// wants its writes accounted for do.
    pub fn settle_task_edits(&mut self) {
        while self.task_edits_outstanding > 0 {
            let Ok(message) = self.task_edit_events.1.recv() else {
                break;
            };
            self.reconcile_task_edit(message);
        }
    }

    fn reconcile_task_edit(&mut self, message: TaskEditMessage) {
        self.task_edits_outstanding = self.task_edits_outstanding.saturating_sub(1);

        match message.result {
            Ok(modified_at) => self.tasks.confirm_edit(message.edit.gid(), modified_at),
            Err(err) => {
                debug_log(&format!("task write failed: {err}"));
                self.task_edit_failures += 1;
                self.task_edit_error = Some(err.to_string());
                // Optimism is worth it — nearly every write succeeds — but an
                // optimistic update that quietly diverges from the server is
                // worse than either, so the rollback is not optional.
                match &message.edit {
                    PendingEdit::Field(edit) => {
                        let rollback = TaskEdit {
                            gid: edit.gid.clone(),
                            field: edit.previous.clone(),
                            previous: edit.field.clone(),
                        };
                        self.tasks.apply_edit_locally(&rollback);
                    }
                    PendingEdit::Project(edit) => {
                        self.tasks.apply_project_edits_locally(&[edit.undo()]);
                    }
                    PendingEdit::Parent(edit) => {
                        self.tasks.apply_parent_edits_locally(&[edit.undo()]);
                    }
                }
            }
        }

        if self.task_edits_outstanding > 0 {
            return;
        }

        if self.task_edit_failures > 0 {
            let error = self
                .task_edit_error
                .clone()
                .unwrap_or_else(|| "unknown error".to_string());
            self.tasks.set_edit_notice(format!(
                "could not update {} of {}: {error}",
                self.task_edit_failures, self.task_edits_sent
            ));
        }
        self.task_edits_sent = 0;
        self.task_edit_failures = 0;
        self.task_edit_error = None;
    }

    pub fn poll_task_data(&mut self) {
        loop {
            let result = {
                let Some(receiver) = self.task_data_receiver.as_ref() else {
                    return;
                };
                receiver.try_recv()
            };

            match result {
                Ok(message) => {
                    if message.generation == self.task_data_generation {
                        if message.done {
                            self.tasks.finish_loading_targets();
                            self.task_data_receiver = None;
                            break;
                        }

                        let Some(project_id) = message.project_id.as_deref() else {
                            continue;
                        };
                        match message.result {
                            Ok(dataset) => {
                                self.tasks
                                    .ingest_loaded_project(project_id, message.query.clone(), dataset)
                            }
                            Err(err) => {
                                debug_log(&format!("task data error: {err}"));
                                self.tasks.set_error(err.to_string());
                                self.task_data_receiver = None;
                                break;
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.task_data_receiver = None;
                    break;
                }
            }
        }
    }

    fn persist_project_visibility(&mut self) -> Result<()> {
        self.config.project_visibility = self.projects.project_visibility_config();
        if self.pending_migration.is_none() {
            self.config.save_to_source_path()?;
        }
        Ok(())
    }

    /// Writes the chart's colour choices back to the config file.
    ///
    /// Only the dimension and its value order: hand-ordering a team is real
    /// work and losing it on exit would be worse than the cost of a write.
    /// Visibility, the column count, and the timeline window are view state of
    /// the same kind as sort and grouping, which have never persisted.
    fn persist_gantt_colors(&mut self) -> Result<()> {
        self.config.gantt.color_by = self.tasks.gantt().color_key().to_string();
        self.config.gantt.order = self.tasks.gantt().orders().clone();
        if self.pending_migration.is_none() {
            self.config.save_to_source_path()?;
        }
        Ok(())
    }

    /// The projects the task data is asked of: the selection, and only the
    /// selection.
    ///
    /// The cursor row used to stand in for an empty selection. That was the
    /// last implicit project list in the app, and it has to go with the
    /// global one: "nothing selected" now means nothing is in play, which is
    /// what makes the empty project state a real state and the filter view's
    /// gate honest. A table drawn from whatever the cursor happened to be
    /// resting on is also the behaviour that made the selection feel
    /// optional, when it is the first clause of every question tuisana asks.
    fn task_target_projects(&self) -> Vec<crate::domain::Project> {
        self.projects.selected_projects()
    }

    fn task_target_project_ids(&self) -> Vec<String> {
        self.task_target_projects()
            .into_iter()
            .map(|project| project.id)
            .collect()
    }

    fn start_task_data_fetch(&mut self) {
        let targets = self.task_target_projects();
        let query_template = self.tasks.desired_task_query();
        let projects_to_load = self.tasks.projects_requiring_load(&targets, &query_template);
        debug_log(&format!(
            "start_task_data_fetch: visible={} scope={:?} targets={} data={}",
            self.tasks.visible(),
            query_template.scope,
            targets
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(","),
            projects_to_load
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        if targets.is_empty() {
            self.tasks
                .finish_loading_dataset(crate::app::task::TaskDataset::default());
            return;
        }

        if projects_to_load.is_empty() {
            self.tasks.begin_loading(&targets);
            self.tasks.finish_loading_targets();
            return;
        }

        self.task_data_generation = self.task_data_generation.wrapping_add(1);
        let generation = self.task_data_generation;
        self.tasks.begin_loading(&targets);

        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.task_data_receiver = Some(receiver);

        thread::spawn(move || {
            for project in projects_to_load {
                let query = TaskQuery { target: TaskTarget::for_project(&project), ..query_template.clone() };
                let result = crate::app::task::TaskState::build_dataset_for_projects(
                    &client,
                    &[project.clone()],
                    &query,
                );
                let _ = sender.send(TaskDataMessage {
                    generation,
                    project_id: Some(project.id.clone()),
                    query: query.clone(),
                    result,
                    done: false,
                });
            }

            let _ = sender.send(TaskDataMessage {
                generation,
                project_id: None,
                query: query_template,
                result: Ok(crate::app::task::TaskDataset::default()),
                done: true,
            });
        });
    }
}

/// The file a config backup goes to, avoiding one that already exists.
///
/// `tuisana.toml` becomes `tuisana.backup.toml`, then
/// `tuisana.backup.2.toml`, and so on: a second migration must not clobber
/// the copy the first one made.
fn backup_path_for(source: Option<&Path>) -> PathBuf {
    let Some(source) = source else {
        return PathBuf::from("tuisana.backup.toml");
    };
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "tuisana".to_string());
    let extension = source
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let directory = source.parent().unwrap_or(Path::new(""));

    let first = directory.join(format!("{stem}.backup{extension}"));
    if !first.exists() {
        return first;
    }
    // Starts at 2 because the unnumbered name is the first one.
    (2u32..)
        .map(|n| directory.join(format!("{stem}.backup.{n}{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

pub fn debug_log(message: &str) {
    if std::env::var_os("TUISANA_DEBUG").is_some() {
        eprintln!("[tuisana] {message}");
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, TaskDto,
                TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto, UserDto,
            },
            fake::FakeAsanaClient,
        },
        config::{Config, ProjectVisibilityConfig, TopPaneState},
        domain::{GanttColorKey, Project},
        input::{Action, AppCommand, KeyBinding},
    };

    use super::{App, PaneSizeState};
    use crate::config::Mode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn app_can_be_created_with_fake_backend_and_load_projects() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items().len(), 1);
        assert_eq!(app.projects.items()[0].name, "Inbox");
        assert_eq!(app.projects.selected_index(), Some(0));
    }

    #[test]
    fn app_handles_project_list_actions() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        assert_eq!(
            app.handle_action(&crate::input::Action::MoveDown, 10)
                .expect("action ok"),
            None
        );
        assert_eq!(app.projects.selected_index(), Some(1));
        assert_eq!(
            app.handle_action(&crate::input::Action::Quit, 10)
                .expect("action ok"),
            Some(crate::input::AppCommand::Quit)
        );
    }

    #[test]
    fn app_applies_project_visibility_config_from_config() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", false),
            Project::new("2", "Backlog", false),
        ]);
        let mut config = Config::default();
        config.project_visibility = vec![ProjectVisibilityConfig {
            gid: "2".to_string(),
            starred: true,
            hidden: true,
        }];
        let mut app = App::new(config, client);

        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items().len(), 2);
        assert_eq!(app.projects.items()[0].id, "2");
        assert_eq!(app.projects.items()[1].id, "1");
        assert_eq!(app.projects.hidden_count(), 2);
    }

    #[test]
    fn app_uses_default_bindings_when_config_does_not_override_them() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "x"
                command = "quit"
            "#,
        )
        .expect("config parses");
        let app = App::new(config, FakeAsanaClient::default());

        let keymap = app.keymap().expect("keymap builds");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::Any),
            Some(&Action::Quit)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('j'), Mode::Any),
            Some(&Action::MoveDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('t'), Mode::Project),
            Some(&Action::SetTaskMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('f'), Mode::Project),
            Some(&Action::SetFilterMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('/'), Mode::Project),
            Some(&Action::StartSearch)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('p'), Mode::Any),
            Some(&Action::SetProjectMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('m'), Mode::Any),
            Some(&Action::ToggleTaskMode)
        );
    }

    #[test]
    fn braces_toggle_the_top_pane_between_restored_minimized_and_maximized() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane");

        assert!(app.panel_size().is_minimized());
        assert_eq!(app.panel_size().actual_height(20, 6), 0);
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("un-minimize top pane");

        assert!(app.panel_size().is_normal());

        app.handle_action(&Action::MaximizeTopPane, 10)
            .expect("maximize top pane");

        assert!(app.panel_size().is_maximized());
        assert_eq!(app.panel_size().actual_height(20, 6), 20);
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::MaximizeTopPane, 10)
            .expect("un-maximize top pane");

        assert!(app.panel_size().is_normal());
    }

    #[test]
    fn switching_to_project_or_filter_mode_restores_a_minimized_top_pane() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane");
        assert!(app.panel_size().is_minimized());
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::SetProjectMode, 10)
            .expect("switch to project mode");
        assert!(app.panel_size().is_normal());
        assert_eq!(app.mode(), Mode::Project);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane again");
        assert!(app.panel_size().is_minimized());
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::SetFilterMode, 10)
            .expect("switch to filter mode");
        assert!(app.panel_size().is_normal());
        assert_eq!(app.mode(), Mode::Filter);
    }

    /// An app with one selected project holding two dated tasks.
    /// The two-task workspace the gantt and view tests share.
    fn gantt_client() -> FakeAsanaClient {
        fn task(gid: &str, name: &str, due: &str) -> TaskDto {
            TaskDto {
                gid: gid.to_string(),
                name: name.to_string(),
                completed: false,
                modified_at: None,
                due_on: Some(due.to_string()),
                start_on: Some("2026-06-01".to_string()),
                assignee: Some(UserDto {
                    gid: format!("u-{gid}"),
                    name: Some(format!("Owner {gid}")),
                    display_name: Some(format!("Owner {gid}")),
                }),
                num_subtasks: 0,
                memberships: vec![TaskMembershipDto {
                    project: TaskMembershipProjectDto {
                        gid: "1".to_string(),
                        name: "Inbox".to_string(),
                    },
                    section: None,
                }],
                parent: None,
                custom_fields: Vec::new(),
            }
        }

        FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]).with_tasks(
            "1",
            vec![task("t1", "Ship it", "2026-07-20"), task("t2", "Pack it", "2026-08-20")],
        )
    }

    fn gantt_app() -> App<FakeAsanaClient> {
        let mut app = App::new(Config::default(), gantt_client());
        app.load_projects().expect("projects load");
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("select the project");
        app
    }

    fn press(app: &mut App<FakeAsanaClient>, code: KeyCode) {
        let keymap = app.keymap().expect("bindings parse");
        app.handle_key_event(&keymap, KeyEvent::new(code, KeyModifiers::NONE), 10)
            .expect("key handled");
    }

    /// `press`, for the keys whose command is the point of the test.
    fn press_with(
        app: &mut App<FakeAsanaClient>,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<AppCommand> {
        let keymap = app.keymap().expect("bindings parse");
        app.handle_key_event(&keymap, KeyEvent::new(code, modifiers), 10)
            .expect("key handled")
    }

    fn settle(app: &mut App<FakeAsanaClient>) {
        for _ in 0..100 {
            app.poll_task_data();
            if !matches!(app.tasks.status(), crate::app::task::TaskStatus::Loading) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn g_draws_the_chart_and_takes_its_controls() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));

        assert_eq!(app.mode(), Mode::Gantt);
        assert!(app.tasks.gantt().visible());
        assert!(app.tasks.visible());
    }

    #[test]
    fn esc_leaves_gantt_mode_with_the_chart_still_drawn() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.mode(), Mode::Task);
        assert!(app.tasks.gantt().visible(), "esc is not a way to hide it");
    }

    #[test]
    fn g_again_hides_the_chart_and_the_mode_goes_with_it() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));

        assert!(!app.tasks.gantt().visible());
        assert_eq!(app.mode(), Mode::Task, "the mode has nothing left to drive");
    }

    #[test]
    fn the_cursor_still_moves_through_tasks_while_in_gantt_mode() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        let before = app.tasks.selected_index();
        press(&mut app, KeyCode::Char('j'));

        assert_ne!(
            app.tasks.selected_index(),
            before,
            "j/k are global bindings and gantt mode falls back to them"
        );
    }

    #[test]
    fn angle_brackets_change_how_many_table_columns_are_visible() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));

        let total = app.tasks.table().columns.len();
        let before = app.tasks.gantt().columns(total);
        press(&mut app, KeyCode::Char('>'));
        assert_eq!(app.tasks.gantt().columns(total), before + 1);

        press(&mut app, KeyCode::Char('<'));
        assert_eq!(app.tasks.gantt().columns(total), before);
    }

    #[test]
    fn c_cycles_the_colour_key_and_wraps() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));

        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Section);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::State);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
    }

    #[test]
    fn c_still_cycles_the_completed_filter_in_task_mode() {
        // `c` is bound in both modes; the mode-specific binding is what makes
        // that safe, so a regression here would be silent.
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        let before = app.tasks.task_settings().filter.completed;
        press(&mut app, KeyCode::Char('c'));

        assert_ne!(app.tasks.task_settings().filter.completed, before);
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
    }

    /// A config backed by a real file, so save_to_source_path has somewhere
    /// to write.
    fn config_on_disk() -> (Config, std::path::PathBuf) {
        config_on_disk_with("")
    }

    /// A path no other test in this process will pick.
    ///
    /// The clock alone is not enough: tests run in parallel, and two that ask
    /// within the same tick would share a config file and fail each other
    /// intermittently.
    fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("tuisana-{prefix}-{nanos}-{seq}.toml"))
    }

    /// A config file holding the header and whatever the test adds to it.
    fn config_on_disk_with(body: &str) -> (Config, std::path::PathBuf) {
        let path = unique_temp_path("config");
        std::fs::write(
            &path,
            format!("[header]\ntype = \"tuisana\"\nversion = 3.0\n{body}"),
        )
        .expect("write config");
        (Config::load_from_path(&path).expect("config loads"), path)
    }


    // ---- Named filter sets -------------------------------------------------

    /// An app in filter mode, fully loaded, with a config file to write to.
    fn filter_sets_app() -> (App<FakeAsanaClient>, std::path::PathBuf) {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter);
        (app, path)
    }

    /// The entries a user saved, in file order — every entry but the app's
    /// own scratch slot, which holds an unnamed panel's selection and is not
    /// one of them.
    fn named_entries(config: &Config) -> Vec<crate::config::NamedFilterSet> {
        config
            .filter_sets
            .iter()
            .filter(|entry| !entry.scratch)
            .cloned()
            .collect()
    }

    fn reread(path: &std::path::Path) -> Config {
        Config::from_toml_str(&std::fs::read_to_string(path).expect("config exists"))
            .expect("the written config reparses")
    }

    /// `n`, and then back into a filter panel.
    ///
    /// `n` starts from nothing, which since version 3 includes the projects —
    /// so it leaves the keys in the project view with nothing selected, and
    /// getting back to a filter panel means picking a project first. That is
    /// the flow the gate exists to force, so the tests go through it.
    fn fresh_panel(app: &mut App<FakeAsanaClient>) {
        press(app, KeyCode::Char('n'));
        assert_eq!(
            app.mode(),
            Mode::Project,
            "`n` clears the selection, which closes the filter view"
        );
        press(app, KeyCode::Char(' '));
        press(app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter, "and a selection reopens it");
    }

    /// `w`, a name, `enter`, clearing whatever the prompt was pre-filled with.
    fn save_as(app: &mut App<FakeAsanaClient>, name: &str) {
        press(app, KeyCode::Char('w'));
        assert_eq!(app.mode(), Mode::FilterSetName);
        for _ in 0..40 {
            press(app, KeyCode::Backspace);
        }
        for ch in name.chars() {
            press(app, KeyCode::Char(ch));
        }
        press(app, KeyCode::Enter);
    }

    /// Moves the field cursor to a row by label, from filter-browse mode.
    fn move_to_field(app: &mut App<FakeAsanaClient>, label: &str) {
        // Back to the top first: `j` only goes one way, and the cursor is
        // shared with wherever the last test step left it.
        for _ in 0..20 {
            press(app, KeyCode::Char('k'));
        }
        for _ in 0..20 {
            let rows = app.tasks.filter_panel_rows();
            let selected = app
                .tasks
                .filter_panel_entries()
                .iter()
                .position(|entry| entry.selected)
                .expect("a row is selected");
            if rows[selected].0 == label {
                return;
            }
            press(app, KeyCode::Char('j'));
        }
        panic!("never reached the {label} row");
    }

    /// `enter`, some text, `enter` — the browse-mode way to fill a text field.
    fn type_into_field(app: &mut App<FakeAsanaClient>, label: &str, text: &str) {
        move_to_field(app, label);
        press(app, KeyCode::Enter);
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
        press(app, KeyCode::Enter);
    }

    #[test]
    fn w_saves_the_panel_under_a_name_and_binds_to_it() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");

        save_as(&mut app, "mine");

        assert_eq!(app.mode(), Mode::Filter, "the prompt closed");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));
        let written = reread(&path);
        let saved = named_entries(&written);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "mine");
        assert_eq!(saved[0].sets[0].fields[0].key, "assignee");
        assert_eq!(saved[0].sets[0].fields[0].query, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_over_an_existing_name_replaces_it_rather_than_adding_a_second() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        type_into_field(&mut app, "Title", "ship");
        // `w` then `enter`: the prompt comes pre-filled with the loaded name,
        // so re-saving where you are takes two keys.
        press(&mut app, KeyCode::Char('w'));
        assert_eq!(app.tasks.filter_set_prompt_text(), Some("mine"));
        press(&mut app, KeyCode::Enter);

        let written = reread(&path);
        let saved = named_entries(&written);
        assert_eq!(saved.len(), 1, "one entry, not two");
        assert_eq!(saved[0].sets[0].fields.len(), 2);

        // A name that differs only in case is the same entry, because the
        // sidebar could not tell the two rows apart.
        save_as(&mut app, "MINE");
        assert_eq!(named_entries(&reread(&path)).len(), 1);
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("MINE"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_name_is_refused_with_the_prompt_still_open() {
        let (mut app, path) = filter_sets_app();

        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::FilterSetName, "still typing");
        assert!(app.tasks.filter_set_prompt_text().is_some());
        assert!(named_entries(&app.config).is_empty());
        assert!(
            app.tasks
                .filter_sets_notice()
                .is_some_and(|notice| notice.contains("name")),
            "and it says why"
        );

        // esc backs out without saving anything.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode(), Mode::Filter);
        assert!(app.tasks.filter_set_prompt_text().is_none());
        assert_eq!(app.tasks.filter_set_loaded_name(), None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_burst_of_typing_into_a_bound_panel_costs_exactly_one_write() {
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "mine");
        let before = reread(&path);

        move_to_field(&mut app, "Assignee");
        press(&mut app, KeyCode::Enter);

        // Mid-burst: the table rebuild is deferred and so is the write.
        app.tasks.set_input_pending(true);
        for ch in "alex".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(
            reread(&path).filter_sets,
            before.filter_sets,
            "nothing written mid-burst"
        );

        // The key that empties the queue writes once, with the whole word.
        app.tasks.set_input_pending(false);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            named_entries(&reread(&path))[0].sets[0].fields[0].query,
            "alexx"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_change_that_changes_nothing_does_not_rewrite_the_config() {
        // The `dirty` flag is deliberately over-eager, so the writer's
        // comparison is the only thing stopping the file from churning.
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        // A marker the app would erase if it rewrote the file.
        let marked = format!(
            "# untouched\n{}",
            std::fs::read_to_string(&path).expect("config exists")
        );
        std::fs::write(&path, &marked).expect("mark the config");

        // `ctrl-l` clears a row that is already clear: a real refresh_table,
        // so the panel is marked dirty, but nothing about it changed.
        move_to_field(&mut app, "Title");
        let keymap = app.keymap().expect("bindings parse");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
            10,
        )
        .expect("key handled");
        // And a plain cursor move, which does not even rebuild.
        press(&mut app, KeyCode::Char('j'));

        assert_eq!(
            std::fs::read_to_string(&path).expect("config exists"),
            marked,
            "the config was rewritten for a no-op change"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn y_copies_the_panel_to_a_new_unnamed_one_and_leaves_the_entry_as_it_was() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        let before = reread(&path);

        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            app.tasks.filter_panel_rows()[1].1,
            "alex",
            "the panel keeps what it was showing"
        );

        type_into_field(&mut app, "Title", "ship");
        assert_eq!(
            reread(&path).filter_sets,
            before.filter_sets,
            "and later edits no longer reach the entry"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn n_starts_a_fresh_panel_without_disturbing_the_entry_it_came_from() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        let before = reread(&path);

        fresh_panel(&mut app);

        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(app.tasks.active_filter_count(), 0, "nothing is set");
        assert_eq!(app.tasks.filter_set_position(), (0, 1), "one empty tab");
        // The named entries, not every entry: the reselection `fresh_panel`
        // makes is an unnamed panel's selection, and that now has somewhere
        // on disk to be.
        assert_eq!(
            named_entries(&reread(&path)),
            named_entries(&before),
            "the entry keeps what was last written to it"
        );

        // And the now-unbound panel writes nothing on a later edit.
        type_into_field(&mut app, "Title", "ship");
        assert_eq!(named_entries(&reread(&path)), named_entries(&before));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_fresh_panel_refetches_what_the_filter_it_dropped_was_hiding() {
        // `n` clears a due filter, which widens the window pushed down to
        // Asana — and a window recorded as covered but never fetched leaves
        // rows missing for good.
        let (mut app, path) = filter_sets_app();

        move_to_field(&mut app, "Due");
        press(&mut app, KeyCode::Enter);
        for ch in "2026-07-01..2026-07-31".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);

        app.tasks.invalidate_cache();
        app.request_task_data().expect("fetch starts");
        settle(&mut app);
        app.poll_task_data();
        assert!(app.task_data_receiver.is_none());

        // `n` also clears the selection, so there is nothing to fetch until
        // something is selected again — which is the moment the widened
        // window has to go to Asana rather than be served from the cache.
        fresh_panel(&mut app);
        assert_eq!(app.tasks.desired_task_query().due_after, None);

        // Asserted on the rows rather than on the receiver: what matters is
        // that the task the narrow window never fetched is on screen, and
        // only a real refetch can put it there.
        settle(&mut app);
        app.poll_task_data();
        app.tasks.settle_table();
        let titles = app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind == crate::domain::TaskRowKind::Task)
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();
        assert!(
            titles.contains(&"Pack it".to_string()),
            "the August task the July window hid has to come back: {titles:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn d_deletes_the_loaded_entry_after_a_confirmation_and_detaches() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode(), Mode::FilterSetName, "it asks first");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(named_entries(&reread(&path)).len(), 1, "`n` keeps it");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.mode(), Mode::Filter);
        assert!(named_entries(&reread(&path)).is_empty());
        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            app.tasks.filter_panel_rows()[1].1,
            "alex",
            "deleting the entry does not empty the panel"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn d_is_refused_when_nothing_is_loaded() {
        // With no cursor in the sidebar there is no other unambiguous target.
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        press(&mut app, KeyCode::Char('y'));

        press(&mut app, KeyCode::Char('d'));

        assert_eq!(app.mode(), Mode::Filter, "no prompt opened");
        assert_eq!(named_entries(&reread(&path)).len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_loads_the_entry_at_that_position_in_the_window() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "bravo");
        // `n` between them, so the second entry starts from nothing rather
        // than inheriting the first one's Assignee.
        fresh_panel(&mut app);
        type_into_field(&mut app, "Title", "ship");
        save_as(&mut app, "alpha");
        fresh_panel(&mut app);

        // Sorted by name, so `1` is alpha and `2` is bravo whatever order
        // they were written in. An empty unnamed panel has nothing to lose,
        // so no confirmation stands in the way.
        press(&mut app, KeyCode::Char('2'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("bravo"));
        assert_eq!(app.tasks.filter_panel_rows()[1].1, "alex");
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "");

        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "ship");

        // A digit past the end of the list does nothing rather than clearing
        // the panel.
        press(&mut app, KeyCode::Char('9'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_asks_before_throwing_away_an_unnamed_panel() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "saved");
        // An unnamed panel with a filter in it: work that exists nowhere but
        // on screen, and the digit did not ask for it to be thrown away.
        fresh_panel(&mut app);
        type_into_field(&mut app, "Title", "unsaved work");

        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.mode(), Mode::FilterSetName, "it asks first");
        assert_eq!(
            app.tasks.filter_panel_rows()[0].1,
            "unsaved work",
            "and nothing has been loaded yet"
        );

        // `n` backs out, leaving the panel exactly as it was.
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "unsaved work");
        assert_eq!(app.tasks.filter_set_loaded_name(), None);

        // `y` goes through with it.
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "");
        assert_eq!(app.tasks.filter_panel_rows()[1].1, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_asks_nothing_when_there_is_nothing_to_lose() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "saved");

        // Bound: every change is already on disk, so loading over it costs
        // nothing.
        assert!(!app.tasks.filter_set_is_unsaved());
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.mode(), Mode::Filter, "no prompt for a bound panel");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));

        // Unnamed but empty: nothing worth a keypress to confirm.
        fresh_panel(&mut app);
        assert!(!app.tasks.filter_set_is_unsaved());
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.mode(), Mode::Filter, "nor for an empty one");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_past_the_end_of_the_list_asks_nothing_and_does_nothing() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Title", "unsaved work");

        press(&mut app, KeyCode::Char('9'));

        assert_eq!(app.mode(), Mode::Filter, "no entry, so no question");
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "unsaved work");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_edit_still_in_hand_reaches_disk_before_the_panel_is_replaced() {
        // The write-through runs *after* the key, by which point a load has
        // already thrown the panel away — so the load has to flush first.
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "alpha");
        fresh_panel(&mut app);
        save_as(&mut app, "bravo");

        move_to_field(&mut app, "Assignee");
        press(&mut app, KeyCode::Enter);
        app.tasks.set_input_pending(true);
        for ch in "alex".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        // Still mid-burst, so committing the edit does not flush it either.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode(), Mode::Filter);
        assert!(
            reread(&path)
                .named_filter_set("bravo")
                .expect("bravo is on disk")
                .sets[0]
                .fields
                .is_empty(),
            "still unwritten, mid-burst"
        );

        // `1` loads alpha. The edit bravo was still holding must not go with
        // the panel it was typed into.
        app.tasks.set_input_pending(false);
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));
        let bravo = reread(&path)
            .filter_sets
            .into_iter()
            .find(|entry| entry.name == "bravo")
            .expect("bravo is still there");
        assert_eq!(bravo.sets[0].fields[0].query, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn b_toggles_the_sidebar_without_taking_the_field_cursor() {
        let (mut app, path) = filter_sets_app();
        let before = app
            .tasks
            .filter_panel_entries()
            .iter()
            .position(|entry| entry.selected);

        press(&mut app, KeyCode::Char('b'));
        assert!(app.tasks.filter_sets_sidebar_visible());

        // `j` still walks the filter fields, which is what the unfocused
        // border is promising.
        press(&mut app, KeyCode::Char('j'));
        let after = app
            .tasks
            .filter_panel_entries()
            .iter()
            .position(|entry| entry.selected);
        assert_eq!(after, before.map(|index| index + 1));

        press(&mut app, KeyCode::Char('b'));
        assert!(!app.tasks.filter_sets_sidebar_visible());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn loading_an_entry_that_widens_the_due_window_starts_a_fetch() {
        // The Milestone 13 hazard, one level up: `TaskQuery::covers` records
        // a fetched window as cached, so a load that skips the fetch decision
        // leaves rows permanently missing rather than merely late.
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "everything");
        // Unbound first, or the due window below would write straight through
        // into the entry this test needs to stay wide.
        fresh_panel(&mut app);

        // A narrow due window, picked on the calendar the way a user would.
        move_to_field(&mut app, "Due");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode(), Mode::Calendar);
        for ch in "2026-07-01..2026-07-31".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);
        save_as(&mut app, "july");

        // Re-fetch so the cache records the narrow window, not the broad one
        // the first load used.
        app.tasks.invalidate_cache();
        app.request_task_data().expect("fetch starts");
        settle(&mut app);
        app.poll_task_data();
        assert!(
            app.task_data_receiver.is_none(),
            "the narrow fetch finished before the load"
        );
        let targets = app.task_target_projects();
        let narrow = app.tasks.desired_task_query();
        assert_eq!(narrow.due_after.as_deref(), Some("2026-07-01"));
        assert!(app.tasks.can_serve_query_for_targets(&targets, &narrow));

        // `everything` sorts before `july`, so `1` is the wider entry. The
        // panel is bound to `july`, so nothing is at risk and no
        // confirmation stands in the way.
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("everything"));
        let widened = app.tasks.desired_task_query();
        assert_eq!(widened.due_after, None, "no due filter left to push down");
        assert_eq!(widened.due_before, None);
        assert!(
            app.task_data_receiver.is_some(),
            "the load has to go through the same fetch decision every other \
             filter change does"
        );

        settle(&mut app);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_config_that_cannot_be_written_reports_instead_of_ending_the_session() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        // A directory where the file should be: the write fails, the session
        // does not.
        std::fs::remove_file(&path).expect("remove the config");
        std::fs::create_dir(&path).expect("put a directory in its way");

        type_into_field(&mut app, "Title", "ship");

        assert!(
            app.tasks
                .filter_sets_notice()
                .is_some_and(|notice| notice.contains("could not save")),
            "the failure is reported"
        );
        assert_eq!(
            app.tasks.filter_panel_rows()[0].1,
            "ship",
            "and the edit still happened"
        );
        // Cleared with the write attempt, so a broken config produces one
        // message rather than one per keystroke.
        assert!(!app.tasks.filter_set_dirty());

        let _ = std::fs::remove_dir(&path);
    }

    // ---- The version-1 migration -------------------------------------------

    /// An app over a version-1 config file, which is what raises the prompt.
    fn migrating_app(body: &str) -> (App<FakeAsanaClient>, std::path::PathBuf) {
        let path = unique_temp_path("migrate");
        std::fs::write(&path, format!("# a hand-written comment\n[header]\ntype = \"tuisana\"\nversion = 1.0\n{body}"))
            .expect("write config");
        let config = Config::load_from_path(&path).expect("config loads");
        (App::new(config, FakeAsanaClient::with_default_projects()), path)
    }

    fn press_key(app: &mut App<FakeAsanaClient>, code: KeyCode) -> Option<AppCommand> {
        let keymap = app.keymap().expect("keymap builds");
        app.handle_key_event(&keymap, KeyEvent::new(code, KeyModifiers::NONE), 10)
            .expect("key handled")
    }

    #[test]
    fn a_version_one_config_asks_before_it_is_touched() {
        let (mut app, path) = migrating_app("");
        let before = std::fs::read_to_string(&path).expect("read back");

        assert!(app.pending_migration_backup().is_some(), "the prompt is up");
        // `stage_view_state` fires on a settled burst of keystrokes, so this
        // is the key that used to rewrite the file behind the prompt.
        press_key(&mut app, KeyCode::Char('j'));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            before,
            "no keystroke may write the config while the prompt is up"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn q_quits_and_leaves_the_file_exactly_as_it_was() {
        let (mut app, path) = migrating_app("");
        let before = std::fs::read_to_string(&path).expect("read back");

        assert_eq!(press_key(&mut app, KeyCode::Char('q')), Some(AppCommand::Quit));
        assert_eq!(std::fs::read_to_string(&path).expect("read back"), before);
        assert!(
            app.pending_migration_backup().is_some(),
            "and the prompt returns next time"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn y_writes_a_byte_for_byte_backup_and_then_migrates() {
        let (mut app, path) = migrating_app("");
        let before = std::fs::read_to_string(&path).expect("read back");
        let backup = app
            .pending_migration_backup()
            .expect("a backup path")
            .to_path_buf();

        press_key(&mut app, KeyCode::Char('y'));

        assert_eq!(
            std::fs::read_to_string(&backup).expect("the backup exists"),
            before,
            "a copy, not a re-serialization: the comments survive"
        );
        let migrated = std::fs::read_to_string(&path).expect("read back");
        assert!(migrated.contains("version = 3.0"), "{migrated}");
        assert!(app.pending_migration_backup().is_none(), "the prompt is answered");

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&backup).ok();
    }

    #[test]
    fn n_migrates_without_a_backup() {
        let (mut app, path) = migrating_app("");
        let backup = app
            .pending_migration_backup()
            .expect("a backup path")
            .to_path_buf();

        press_key(&mut app, KeyCode::Char('n'));

        assert!(!backup.exists(), "no backup was asked for");
        assert!(std::fs::read_to_string(&path)
            .expect("read back")
            .contains("version = 3.0"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_existing_backup_is_not_clobbered() {
        let (app, path) = migrating_app("");
        let first = app
            .pending_migration_backup()
            .expect("a backup path")
            .to_path_buf();
        assert!(first.to_string_lossy().ends_with(".backup.toml"));
        std::fs::write(&first, "the backup from last time").expect("write a backup");

        // A second session over the same file resolves the next free name,
        // and the window names the file it will actually write.
        let (app, _) = (
            App::new(
                Config::load_from_path(&path).expect("config loads"),
                FakeAsanaClient::with_default_projects(),
            ),
            (),
        );
        let second = app.pending_migration_backup().expect("a backup path");
        assert!(
            second.to_string_lossy().ends_with(".backup.2.toml"),
            "{second:?}"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&first).ok();
    }

    #[test]
    fn a_config_already_at_version_two_prompts_for_nothing() {
        let (config, path) = config_on_disk();
        let app = App::new(config, FakeAsanaClient::with_default_projects());

        assert!(app.pending_migration_backup().is_none());
        std::fs::remove_file(&path).ok();
    }

    // ---- Persisted view state ----------------------------------------------

    /// An app over two projects, backed by a config file with the given body.
    ///
    /// Built the way the binary builds it — config first, then
    /// `load_projects` — because that order is what the restore depends on.
    fn view_app(body: &str) -> (App<FakeAsanaClient>, std::path::PathBuf) {
        let (config, path) = config_on_disk_with(body);
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Website", true),
        ]);
        let mut app = App::new(config, client);
        app.load_projects().expect("projects load");
        (app, path)
    }

    /// The `[[filter_set]]` an unnamed panel's selection lives in from
    /// version 3 on, as config text a fixture can append.
    fn scratch_body(projects: &[&str]) -> String {
        let list = projects
            .iter()
            .map(|gid| format!("\"{gid}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("\n[[filter_set]]\nname = \"unnamed\"\nscratch = true\nprojects = [{list}]\n")
    }

    /// An app whose scratch entry holds `projects`, which is what an unnamed
    /// panel's selection looks like on disk from version 3 on.
    fn scratch_app(projects: &[&str]) -> (App<FakeAsanaClient>, std::path::PathBuf) {
        view_app(&scratch_body(projects))
    }

    /// The gids the app would restore, in the order it writes them.
    fn selected_gids(app: &App<FakeAsanaClient>) -> Vec<String> {
        let mut gids = app
            .projects
            .selected_projects()
            .into_iter()
            .map(|project| project.id)
            .collect::<Vec<_>>();
        gids.sort();
        gids
    }

    #[test]
    fn opening_a_pane_writes_it_to_the_config() {
        let (mut app, path) = view_app("");
        assert!(
            !std::fs::read_to_string(&path)
                .expect("config exists")
                .contains("[view]"),
            "an app that has opened nothing writes no view section"
        );

        press(&mut app, KeyCode::Char('t'));
        assert!(reread(&path).view.tasks, "the task pane is open");

        // The filter view needs a selection to be asked of.
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('f'));
        let view = reread(&path).view;
        assert!(view.filters, "and the filter panel is up top");
        assert!(view.tasks, "which does not close the table underneath");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_view_reopens_the_same_panes() {
        // With a selection: the filter view is unreachable without one, so a
        // restored `filters = true` has to have something to be asked of.
        let (app, path) = view_app(&format!(
            "\n[view]\ntasks = true\nfilters = true\n{}",
            scratch_body(&["1"])
        ));

        assert_eq!(app.mode(), Mode::Filter);
        assert!(app.tasks.visible());
        assert!(app.tasks.filter_panel_visible());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_top_pane_comes_back_maximized_but_not_at_its_old_height() {
        // From project mode: `]` and `}` are the top pane's keys there, and
        // mean something else entirely once the table has focus.
        let (mut app, path) = view_app("");
        // Two resizes and then a maximize: the height is view state the file
        // deliberately does not keep, the maximize is not.
        press(&mut app, KeyCode::Char(']'));
        press(&mut app, KeyCode::Char(']'));
        press(&mut app, KeyCode::Char('}'));

        let written = reread(&path);
        assert_eq!(written.view.top_pane, TopPaneState::Maximized);
        let serialized = std::fs::read_to_string(&path).expect("config exists");
        assert!(
            !serialized.contains("preferred") && !serialized.contains("height"),
            "no pane height is written: {serialized}"
        );

        let restored = App::new(written, FakeAsanaClient::new(Vec::new()));
        assert!(restored.panel_size().is_maximized());
        assert_eq!(
            restored.panel_size().preferred(),
            PaneSizeState::default().preferred(),
            "the height it was resized to is not restored"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_minimized_top_pane_comes_back_with_the_table_focused() {
        // `{` minimizes *and* moves to the table, so the pair is the only
        // state the file can hold; restoring the size without the focus
        // would leave the keys going somewhere invisible.
        let (app, path) = view_app("\n[view]\ntasks = true\ntop_pane = \"minimized\"\n");

        assert!(app.panel_size().is_minimized());
        assert_eq!(app.mode(), Mode::Task);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_selected_projects_are_written_and_come_back() {
        let (mut app, path) = view_app("");
        press(&mut app, KeyCode::Char(' ')); // select Inbox, cursor moves on
        press(&mut app, KeyCode::Char(' ')); // select Website
        assert_eq!(selected_gids(&app), ["1", "2"]);
        // Into the scratch entry, because the panel is unnamed: since
        // version 3 a selection has nowhere else to go.
        assert_eq!(reread(&path).scratch_projects(), ["1", "2"]);

        let (restored, other) = scratch_app(&["2", "1"]);
        assert_eq!(selected_gids(&restored), ["1", "2"]);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&other);
    }

    #[test]
    fn a_project_the_workspace_no_longer_returns_is_dropped_from_the_selection() {
        // Same rule a reload already applies to the live selection: a project
        // someone left must not keep asking to be loaded.
        let (app, path) = scratch_app(&["1", "gone"]);

        assert_eq!(selected_gids(&app), ["1"]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_refresh_keeps_the_live_selection_rather_than_restoring_the_saved_one() {
        let (mut app, path) = scratch_app(&["1"]);
        assert_eq!(selected_gids(&app), ["1"]);

        app.projects.clear_selection();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' ')); // Website instead
        app.refresh().expect("refresh");

        assert_eq!(
            selected_gids(&app),
            ["2"],
            "the restore happens once, at startup"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_bound_filter_set_is_reloaded_and_still_bound() {
        let (app, path) = view_app(
            "\n[view]\nfilters = true\nfilter_set = \"mine\"\n\n\
             [[filter_set]]\nname = \"mine\"\n\n\
             [[filter_set.set]]\n\n\
             [[filter_set.set.field]]\nkey = \"assignee\"\nquery = \"alex\"\n",
        );

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));
        assert!(
            !app.tasks.filter_set_dirty(),
            "freshly loaded is what is already on disk"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_restored_view_filters_the_table_it_reopens() {
        // The whole feature in one run: the panes, the selection, and the
        // bound entry come back, the fetch the selection implies starts on
        // its own, and the filter — parked while there were no rows to put it
        // on — lands when the data does.
        let (config, path) = config_on_disk_with(
            "\n[view]\ntasks = true\nfilter_set = \"shipping\"\n\n\
             [[filter_set]]\nname = \"shipping\"\nprojects = [\"1\"]\n\n\
             [[filter_set.set]]\n\n\
             [[filter_set.set.field]]\nkey = \"title\"\nquery = \"ship\"\n",
        );
        let mut app = App::new(config, gantt_client());
        app.load_projects().expect("projects load");

        assert!(app.tasks.visible());
        assert_eq!(selected_gids(&app), ["1"]);
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("shipping"));

        settle(&mut app);
        app.tasks.settle_table();

        let titles = app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind == crate::domain::TaskRowKind::Task)
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["Ship it"], "the saved title filter applied");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unnamed_panel_records_no_filter_set_and_a_deleted_name_is_ignored() {
        let (mut app, path) = view_app("");
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(reread(&path).view.filter_set, None);

        // A `[view]` pointing at an entry that is no longer there loads
        // nothing rather than refusing to start.
        let (unbound, other) =
            view_app("\n[view]\nfilters = true\nfilter_set = \"long gone\"\n");
        assert_eq!(unbound.tasks.filter_set_loaded_name(), None);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&other);
    }

    #[test]
    fn the_sidebar_and_the_recent_pane_keep_their_toggles() {
        let (mut app, path) = view_app("");
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('f'));
        press(&mut app, KeyCode::Char('b')); // the Sets sidebar
        assert!(app.tasks.filter_sets_sidebar_visible());

        press(&mut app, KeyCode::Char('t')); // back to the table
        press(&mut app, KeyCode::Char('b')); // and off with the recent pane
        assert!(!app.tasks.recent_pane_enabled());

        let written = reread(&path);
        assert!(written.view.filter_sidebar);
        assert!(!written.view.recent);

        let restored = App::new(written, FakeAsanaClient::new(Vec::new()));
        assert!(restored.tasks.filter_sets_sidebar_visible());
        assert!(!restored.tasks.recent_pane_enabled());

        let _ = std::fs::remove_file(&path);
    }

    // ---- A filter set carries its projects ---------------------------------

    /// The config body a `[[filter_set]]` test starts from: one entry, with
    /// whatever `projects` the test is about.
    fn projects_entry(projects: &str) -> String {
        format!("\n[[filter_set]]\nname = \"web\"\n{projects}\n")
    }

    #[test]
    fn a_digit_in_the_project_view_replaces_the_selection_and_stays_there() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"2\"]"));
        assert_eq!(app.mode(), Mode::Project);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);

        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(
            app.mode(),
            Mode::Project,
            "the key was pressed in the project view and the keys stay there"
        );
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("web"));
        assert_eq!(selected_gids(&app), vec!["2".to_string()]);

        // A digit is a keypress in the session, so the selection half of the
        // load is recoverable even though the filter half is not.
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);
        press_with(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(selected_gids(&app), vec!["2".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    /// An entry with no `projects` key is one migrated from an older file.
    /// There is no global selection left for it to inherit, so it loads to
    /// nothing selected — which is the state that forces a choice.
    #[test]
    fn an_entry_with_no_projects_key_loads_to_nothing_selected() {
        let (mut app, path) = view_app(&projects_entry(""));
        press(&mut app, KeyCode::Char(' '));

        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("web"));
        assert!(selected_gids(&app).is_empty());
        assert_eq!(app.mode(), Mode::Project, "and the keys are where the fix is");

        // Recoverable, like every other selection change a keypress makes.
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    /// The gate, from the other side: `f` says why rather than opening a
    /// panel with no projects behind it.
    #[test]
    fn f_is_refused_while_nothing_is_selected_and_says_why() {
        let (mut app, path) = view_app("");

        press(&mut app, KeyCode::Char('f'));

        assert_eq!(app.mode(), Mode::Project);
        assert!(!app.tasks.filter_panel_visible());
        let notice = app.tasks.edit_notice().expect("it says why");
        assert!(notice.contains("No projects selected"), "{notice}");

        // And a selection is all it was waiting for.
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter);
        assert!(app.tasks.filter_panel_visible());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_projects_list_loads_to_nothing_selected() {
        // Absent is "no opinion"; `[]` is an opinion that selects nothing.
        let (mut app, path) = view_app(&projects_entry("projects = []"));
        press(&mut app, KeyCode::Char(' '));

        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        assert!(selected_gids(&app).is_empty());
        assert!(app.projects.can_undo_selection());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_selection_change_while_bound_rewrites_the_entry() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"2\"]"));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["2".to_string()].as_slice()),
            "loading alone writes nothing"
        );

        // `space` on a project while `web` is loaded edits `web`, the same
        // way typing into a filter field does.
        press(&mut app, KeyCode::Char(' '));

        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["1".to_string(), "2".to_string()].as_slice()),
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The intended upgrade path: the key appears when you first express an
    /// opinion through it.
    #[test]
    fn an_entry_with_no_projects_key_gains_one_when_the_selection_moves() {
        let (mut app, path) = view_app(&projects_entry(""));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(reread(&path).filter_sets[0].projects, None);

        press(&mut app, KeyCode::Char(' '));

        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["1".to_string()].as_slice()),
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Applying a set is not consent to rewrite it: a project you lost access
    /// to for a week should not cost you the entry.
    #[test]
    fn a_gid_the_workspace_no_longer_returns_stays_on_disk_until_the_selection_moves() {
        let (mut app, path) = view_app(&format!(
            "\n[view]\nfilter_set = \"web\"\n{}",
            projects_entry("projects = [\"2\", \"9999\"]")
        ));
        assert_eq!(selected_gids(&app), vec!["2".to_string()], "9999 is skipped");

        // Any number of keys that are not selection changes: the entry is
        // bound, and staying silent is what leaves the gid alone.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["2".to_string(), "9999".to_string()].as_slice()),
        );

        // A deliberate edit of the selection is what removes it.
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["1".to_string(), "2".to_string()].as_slice()),
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_burst_of_selection_keys_costs_one_write_that_also_moves_the_view() {
        let (mut app, path) = view_app(&projects_entry("projects = []"));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        // A marker the app would erase if it rewrote the file mid-burst.
        let marked = format!(
            "# untouched\n{}",
            std::fs::read_to_string(&path).expect("config exists")
        );
        std::fs::write(&path, &marked).expect("mark the config");

        app.tasks.set_input_pending(true);
        press(&mut app, KeyCode::Char('a')); // select every visible project
        press(&mut app, KeyCode::Char('i')); // invert it
        assert_eq!(
            std::fs::read_to_string(&path).expect("config exists"),
            marked,
            "nothing written mid-burst"
        );

        // The key that empties the queue writes once, and that one write
        // moves both `[[filter_set]]` and `[view]`.
        app.tasks.set_input_pending(false);
        press(&mut app, KeyCode::Char('a'));

        let written = reread(&path);
        assert_eq!(
            written.filter_sets[0].projects.as_deref(),
            Some(["1".to_string(), "2".to_string()].as_slice()),
        );
        assert_eq!(
            written.view.filter_set.as_deref(),
            Some("web"),
            "and the same write recorded the binding"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn n_clears_the_selection_and_u_puts_it_back() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"1\", \"2\"]"));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(selected_gids(&app), vec!["1".to_string(), "2".to_string()]);

        press(&mut app, KeyCode::Char('n'));

        assert!(selected_gids(&app).is_empty(), "`n` starts from nothing");
        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["1".to_string(), "2".to_string()].as_slice()),
            "the entry keeps whatever was last written to it"
        );

        press(&mut app, KeyCode::Char('u'));
        assert_eq!(selected_gids(&app), vec!["1".to_string(), "2".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn w_writes_the_live_selection_into_the_entry_it_saves() {
        let (mut app, path) = view_app("");
        press(&mut app, KeyCode::Char(' '));
        save_as(&mut app, "mine");

        assert_eq!(app.mode(), Mode::Project, "the prompt gives the keys back");
        assert_eq!(
            reread(&path).filter_sets[0].projects.as_deref(),
            Some(["1".to_string()].as_slice()),
            "`w` always writes the key, so a saved entry is unambiguous",
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_entry_saved_with_nothing_selected_records_an_empty_list() {
        let (mut app, path) = view_app("");
        save_as(&mut app, "mine");

        assert_eq!(reread(&path).filter_sets[0].projects, Some(Vec::new()));

        let _ = std::fs::remove_file(&path);
    }

    /// A bound entry's `projects` wins over `[view].projects`, because the
    /// filter *fields* already come back from the entry rather than from
    /// `[view]`.
    #[test]
    fn at_startup_a_bound_entrys_projects_beat_the_view() {
        let (app, path) = view_app(&format!(
            "\n[view]\nprojects = [\"1\"]\nfilter_set = \"web\"\n{}",
            projects_entry("projects = [\"2\"]")
        ));

        assert_eq!(selected_gids(&app), vec!["2".to_string()]);
        assert!(
            !app.projects.can_undo_selection(),
            "restoring the view is not an edit you made this session"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A bound entry that names no projects has nothing to fall back to, so
    /// the session opens in the state that asks for a selection.
    #[test]
    fn at_startup_an_entry_with_no_projects_key_restores_nothing() {
        let (app, path) = view_app(&format!(
            "\n[view]\nfilters = true\nfilter_set = \"web\"\n{}",
            projects_entry("")
        ));

        assert!(selected_gids(&app).is_empty());
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("web"));
        assert_eq!(
            app.mode(),
            Mode::Project,
            "a restored filter view gives way when it turns out to have no projects"
        );
        assert!(!app.tasks.filter_panel_visible());

        let _ = std::fs::remove_file(&path);
    }

    /// The scratch entry is where an unnamed panel's selection lives, so it
    /// is what an unnamed panel comes back to.
    #[test]
    fn an_unnamed_panel_restores_from_the_scratch_entry() {
        let (app, path) = scratch_app(&["2"]);

        assert_eq!(selected_gids(&app), vec!["2".to_string()]);
        assert_eq!(app.tasks.filter_set_loaded_name(), None);

        let _ = std::fs::remove_file(&path);
    }

    /// A bound name wins over the scratch slot, and a bound name whose entry
    /// has been deleted restores nothing rather than inheriting a selection
    /// that belonged to a different panel.
    #[test]
    fn a_bound_entry_beats_the_scratch_slot_and_a_missing_one_takes_neither() {
        let (app, path) = view_app(&format!(
            "\n[view]\nfilter_set = \"web\"\n{}{}",
            projects_entry("projects = [\"1\"]"),
            scratch_body(&["2"])
        ));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);
        let _ = std::fs::remove_file(&path);

        let (app, path) = view_app(&format!(
            "\n[view]\nfilter_set = \"deleted by hand\"\n{}",
            scratch_body(&["2"])
        ));
        assert!(selected_gids(&app).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// The round trip the scratch slot exists for: select with nothing bound,
    /// quit, come back to the same projects.
    #[test]
    fn an_unnamed_selection_survives_a_restart() {
        let (mut app, path) = view_app("");
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);

        let written = reread(&path);
        assert_eq!(written.scratch_projects(), ["1"]);
        assert!(
            written.sorted_filter_sets().is_empty(),
            "and the sidebar still lists nothing"
        );

        let mut restored = App::new(written, FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Website", true),
        ]));
        restored.load_projects().expect("projects load");
        assert_eq!(selected_gids(&restored), vec!["1".to_string()]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_sidebar_opened_in_the_project_view_is_still_open_after_f_and_p() {
        let (mut app, path) = view_app("");

        press(&mut app, KeyCode::Char('b'));
        assert!(app.tasks.filter_sets_sidebar_visible());

        press(&mut app, KeyCode::Char('f'));
        assert!(
            app.tasks.filter_sets_sidebar_visible(),
            "switching views does not open or close it"
        );
        press(&mut app, KeyCode::Char('p'));
        assert!(app.tasks.filter_sets_sidebar_visible());
        assert_eq!(app.mode(), Mode::Project);

        // And `b` in the project view closes it again.
        press(&mut app, KeyCode::Char('b'));
        assert!(!app.tasks.filter_sets_sidebar_visible());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_load_with_the_task_pane_closed_marks_the_table_out_of_date() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"2\"]"));
        press(&mut app, KeyCode::Char(' '));
        // The table has to have held something for staleness to be worth
        // reporting — an idle one has nothing to be out of date — but the
        // pane itself stays closed, which is the case under test.
        let inbox = [Project::new("1", "Inbox", true)];
        let table =
            crate::app::task::TaskState::build_table_for_projects(&app.client, &inbox)
                .expect("table builds");
        app.tasks.begin_loading(&inbox);
        app.tasks.finish_loading(table);
        assert!(!app.tasks.visible());

        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(selected_gids(&app), vec!["2".to_string()]);
        assert!(
            matches!(
                app.tasks.status(),
                crate::app::task::TaskStatus::OutOfDate(_)
            ),
            "a closed table is marked stale rather than fetched behind its back: {:?}",
            app.tasks.status(),
        );
        assert!(app.task_data_receiver.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn y_and_d_act_from_the_project_view_and_give_the_keys_back() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"2\"]"));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        // `y` keeps what is on screen, selection included, and unbinds.
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(selected_gids(&app), vec!["2".to_string()]);

        // `d` is refused with nothing loaded, then deletes once there is.
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode(), Mode::Project, "nothing to delete, nothing asked");

        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode(), Mode::FilterSetName, "it asks first");
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.mode(), Mode::Project, "and the keys come back here");
        assert!(named_entries(&reread(&path)).is_empty());
        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            selected_gids(&app),
            vec!["2".to_string()],
            "the panel keeps what it is showing, selection included"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_page_keys_move_the_window_the_digits_address_from_the_project_view() {
        let mut body = String::new();
        for index in 0..12 {
            body.push_str(&format!("\n[[filter_set]]\nname = \"set {index:02}\"\n"));
        }
        let (mut app, path) = view_app(&body);
        press(&mut app, KeyCode::Char('b'));
        app.tasks.set_filter_sets_window(9);

        press(&mut app, KeyCode::Char('>'));
        assert_eq!(app.tasks.filter_sets_page_start(), 9);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("set 09"));

        press(&mut app, KeyCode::Char('<'));
        assert_eq!(app.tasks.filter_sets_page_start(), 0);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("set 00"));

        let _ = std::fs::remove_file(&path);
    }

    /// A star is a fact about a project; a selection is a fact about a
    /// question. The four keys that are about the list write nothing to the
    /// entry the selection belongs to.
    #[test]
    fn visibility_and_list_view_keys_write_nothing_to_the_bound_entry() {
        let (mut app, path) = view_app(&projects_entry("projects = [\"2\"]"));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('1'));

        // Both projects start starred and hidden, so each toggle flips one
        // flag off and the written record is the flip.
        press(&mut app, KeyCode::Char('*')); // star the selection
        press(&mut app, KeyCode::Char('v')); // show the hidden group
        press(&mut app, KeyCode::Char('o')); // selected only
        press(&mut app, KeyCode::Char('h')); // hide the selection

        let written = reread(&path);
        assert_eq!(
            written.filter_sets[0].projects.as_deref(),
            Some(["2".to_string()].as_slice()),
            "the entry is untouched"
        );
        let flags = |gid: &str| {
            written
                .project_visibility
                .iter()
                .find(|project| project.gid == gid)
                .map(|project| (project.starred, project.hidden))
        };
        assert_eq!(
            flags("2"),
            Some((false, false)),
            "the facts about the project went to `[[project]]`: {:?}",
            written.project_visibility
        );
        assert_eq!(flags("1"), Some((true, true)), "and only the selected one moved");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_project_mode_keys_keep_every_meaning_they_had() {
        // Seven new keys in a view that already had plenty: `a`, `i`, `c`,
        // `u`, `h`, `v`, and `o` are the ones a slip here would break.
        let (mut app, path) = view_app("");

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(selected_gids(&app), vec!["1".to_string(), "2".to_string()]);
        press(&mut app, KeyCode::Char('i'));
        assert!(selected_gids(&app).is_empty());
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('c'));
        assert!(selected_gids(&app).is_empty());
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(selected_gids(&app), vec!["1".to_string()]);
        press(&mut app, KeyCode::Char('o'));
        assert!(app.projects.show_selected_only());
        press(&mut app, KeyCode::Char('v'));
        assert!(app.projects.hidden_visible());
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.projects.hidden_count(), 1, "`h` still hides");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_dialog_opens_over_the_current_dimensions_values() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::GanttOrder);
        let dialog = app.tasks.gantt().dialog().expect("the dialog is open");
        assert_eq!(dialog.key(), &GanttColorKey::Assignee);
        assert_eq!(dialog.entries().len(), 2, "two assignees, none unassigned");
    }

    #[test]
    fn j_and_k_drive_the_dialog_rather_than_the_task_rows() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        let row = app.tasks.selected_index();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));

        assert_eq!(app.tasks.gantt().dialog().expect("open").selected(), 1);
        assert_eq!(app.tasks.selected_index(), row, "the table did not move");
    }

    #[test]
    fn saving_the_dialog_writes_the_order_to_the_config_file() {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b')); // send the first value to the bottom
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::Gantt);
        let written = std::fs::read_to_string(&path).expect("config was written");
        let reloaded = Config::from_toml_str(&written).expect("it reparses");
        assert_eq!(
            reloaded.gantt.order_for(&GanttColorKey::Assignee),
            ["Owner t2".to_string(), "Owner t1".to_string()],
        );
        assert_eq!(reloaded.gantt.color_by, "assignee");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cancelling_the_dialog_writes_nothing() {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.mode(), Mode::Gantt);
        assert!(!app.tasks.gantt().dialog_open());
        // Opening the task pane persists `[view]`, so the file has moved —
        // what the cancel must leave untouched is the colour order.
        let reloaded =
            Config::from_toml_str(&std::fs::read_to_string(&path).expect("config exists"))
                .expect("it reparses");
        assert!(reloaded.gantt.order.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_chart_visibility_and_column_count_are_not_persisted() {
        // View state of the same kind as sort and grouping, which have never
        // persisted. Only the colour choices are the user's own work.
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('>'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);

        let written = std::fs::read_to_string(&path).expect("config was written");
        let reloaded = Config::from_toml_str(&written).expect("it reparses");
        assert!(!reloaded.gantt.visible);
        assert_eq!(reloaded.gantt.columns, 2, "the default, not the session's 3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn changing_project_visibility_after_using_the_chart_writes_no_gantt_table() {
        // save_to_source_path rewrites the whole file, so an untouched section
        // must not start appearing in the user's config just because the chart
        // was opened.
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('>'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('p'));
        // Hiding rather than starring: the fixture project starts starred, so
        // a star toggle would return it to the default and write nothing.
        press(&mut app, KeyCode::Char('h'));

        let written = std::fs::read_to_string(&path).expect("config was written");
        assert!(
            written.contains("[[project]]"),
            "the visibility change was saved: {written}"
        );
        assert!(!written.contains("[gantt]"), "{written}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_colour_order_is_in_effect_after_a_restart() {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Enter);

        let reloaded = Config::load_from_path(&path).expect("config reloads");
        let restarted = App::new(reloaded, FakeAsanaClient::new(Vec::new()));

        assert_eq!(
            restarted.tasks.gantt().order(),
            ["Owner t2".to_string(), "Owner t1".to_string()],
            "no hand-editing needed between sessions"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn config_can_start_the_session_with_the_chart_already_drawn() {
        let mut config = Config::default();
        config.gantt.visible = true;
        let app = App::new(config, FakeAsanaClient::new(Vec::new()));

        assert!(app.tasks.gantt().visible());
    }

    #[test]
    fn pressing_t_in_project_mode_switches_to_task_mode() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        assert_eq!(app.mode(), Mode::Project);

        app.handle_action(&Action::SetTaskMode, 10)
            .expect("set task mode");

        assert_eq!(app.mode(), Mode::Task);
        assert!(app.tasks.visible());
    }

    #[test]
    fn r_still_triggers_refresh() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        let keymap = app.keymap().expect("keymap builds");

        let command = app
            .handle_key_event(
                &keymap,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                10,
            )
            .expect("refresh command");

        assert_eq!(command, Some(crate::input::AppCommand::Refresh));
    }

    /// Once a project's tasks are loaded, the cache reports the query as
    /// already covered — so a naive refresh that only checks
    /// `task_data_needs_refresh` would see nothing to do and skip fetching
    /// entirely. `App::refresh` has to invalidate that cache itself.
    #[test]
    fn refresh_reloads_tasks_even_when_the_cache_already_covers_the_query() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.handle_action(&Action::SelectAllVisible, 10)
            .expect("select every project");
        app.tasks.set_visible(true);
        app.request_task_data().expect("tasks load");
        settle(&mut app);

        let targets = app.task_target_projects();
        let query = app.tasks.desired_task_query();
        assert!(
            app.tasks.can_serve_query_for_targets(&targets, &query),
            "cache should already cover the loaded selection"
        );

        app.refresh().expect("refresh");

        assert_eq!(
            app.tasks.status(),
            &crate::app::task::TaskStatus::Loading,
            "refresh must force a fetch instead of trusting the cache"
        );
    }

    #[test]
    fn app_marks_tasks_out_of_date_when_project_selection_changes() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let table = crate::app::task::TaskState::build_table_for_projects(
            &app.client,
            &[Project::new("1", "Inbox", true)],
        )
        .expect("table builds");
        app.tasks.begin_loading(&[Project::new("1", "Inbox", true)]);
        app.tasks.finish_loading(table);

        assert_eq!(
            app.tasks.status(),
            &crate::app::task::TaskStatus::Empty
        );

        app.handle_action(&Action::MoveDown, 10).expect("move down");
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("toggle selection");

        assert!(matches!(
            app.tasks.status(),
            crate::app::task::TaskStatus::OutOfDate(message)
                if message.contains("switch to task view")
        ));
    }

    #[test]
    fn page_navigation_uses_the_task_view_when_it_is_visible() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_custom_field_settings(
                "1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "cfs1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Task 1".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Task 2".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t3".to_string(),
                        name: "Task 3".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t4".to_string(),
                        name: "Task 4".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                ],
            );
        let table = crate::app::task::TaskState::build_table_for_projects(
            &client,
            &[Project::new("1", "Inbox", true)],
        )
        .expect("tasks load");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks.begin_loading(&[Project::new("1", "Inbox", true)]);
        app.tasks.finish_loading(table);
        app.tasks.set_visible(true);

        app.handle_action(&Action::PageDown, 2).expect("page down");
        assert_eq!(app.tasks.selected_index(), Some(6));

        app.handle_action(&Action::PageUp, 2).expect("page up");
        assert_eq!(app.tasks.selected_index(), Some(4));
    }

    #[test]
    fn task_view_bindings_override_project_bindings_when_tasks_are_visible() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "c"
                command = "clear_selection"
            "#,
        )
        .expect("config parses");

        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Open task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Closed task".to_string(),
                        completed: true,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![],
                    },
                ],
            );

        let mut app = App::new(config, client);
        app.load_projects().expect("projects load");
        app.tasks.set_completed_filter(None);
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
            .expect("tasks load");
        app.tasks.set_visible(true);

        assert_eq!(app.tasks.table().task_count(), 2);
        app.handle_action(&Action::ToggleCompletedFilter, 10)
            .expect("task filter action");
        assert_eq!(app.tasks.table().task_count(), 1);
        assert!(app.tasks.filter_summary().contains("comp open"));
    }

    #[test]
    fn the_filter_panel_stays_up_when_the_keys_move_to_the_table() {
        let mut app = gantt_app();
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter);

        press(&mut app, KeyCode::Char('t'));

        assert_eq!(app.mode(), Mode::Task);
        assert!(
            app.tasks.filter_panel_visible(),
            "the filters you just built are what you are reading the table \
             against"
        );
        assert!(!app.filter_panel_focused(), "but the keys are on the table");

        // `p` is the key that means "show me the projects", and it is what
        // puts the top pane back.
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.mode(), Mode::Project);
        assert!(!app.tasks.filter_panel_visible());
    }

    #[test]
    fn the_table_keeps_its_cursor_keys_while_the_filter_panel_is_showing() {
        let mut app = gantt_app();
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        press(&mut app, KeyCode::Char('t'));

        let filter_row_before = app
            .tasks
            .filter_panel_entries()
            .iter()
            .position(|entry| entry.selected);
        let task_row_before = app.tasks.selected_index();

        press(&mut app, KeyCode::Char('j'));

        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|entry| entry.selected),
            filter_row_before,
            "the panel is on screen but is not the thing being driven"
        );
        assert_ne!(app.tasks.selected_index(), task_row_before);

        // And `f` hands them back without having to reopen anything.
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter);
        assert!(app.tasks.filter_panel_visible());
        press(&mut app, KeyCode::Char('j'));
        assert_ne!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|entry| entry.selected),
            filter_row_before
        );
    }

    #[test]
    fn a_letter_types_into_the_table_not_the_panel_once_the_keys_have_left_it() {
        let mut app = gantt_app();
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        press(&mut app, KeyCode::Enter); // open the Title row
        for ch in "ship".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        // `t` is one of those letters: filter-edit mode does not fall back to
        // the global bindings, which is why leaving takes `enter` first.
        assert_eq!(app.mode(), Mode::FilterEdit);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('t'));

        assert_eq!(app.mode(), Mode::Task);
        assert!(!app.tasks.filter_panel_editing());
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "ship", "the filter is kept");

        // Raw characters used to reach the panel whenever it was visible.
        // Now they follow the keys: `c` is the table's completed filter.
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "ship");
        assert!(app.tasks.filter_summary().contains("comp"));
    }

    /// The whole path a cut takes: the bound key, the buffer it edits, and the
    /// command that carries what it took out to the clipboard.
    #[test]
    fn the_cut_keys_edit_the_filter_field_and_hand_the_text_to_the_clipboard() {
        let mut app = gantt_app();
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        press(&mut app, KeyCode::Enter); // open the Title row
        for ch in "ship it".chars() {
            press(&mut app, KeyCode::Char(ch));
        }

        // `alt-b` back over "it", then `ctrl-k` takes the rest of the line.
        press_with(&mut app, KeyCode::Char('b'), KeyModifiers::ALT);
        assert_eq!(
            press_with(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL),
            Some(AppCommand::CopyToClipboard("it".to_string()))
        );
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "ship ");

        // Nothing ahead of the caret: no command, so the clipboard keeps what
        // the cut above put there.
        assert_eq!(
            press_with(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL),
            None
        );

        press_with(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(
            press_with(&mut app, KeyCode::Char('d'), KeyModifiers::ALT),
            Some(AppCommand::CopyToClipboard("ship".to_string()))
        );
        assert_eq!(app.tasks.filter_panel_rows()[0].1, " ");
    }

    #[test]
    fn toggle_task_filters_closes_the_panel_when_it_is_already_visible() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks.set_visible(true);
        app.tasks.toggle_filter_panel();
        assert!(app.tasks.filter_panel_visible());

        app.handle_action(&Action::ToggleTaskFilters, 10)
            .expect("toggle filters");

        assert!(!app.tasks.filter_panel_visible());
    }

    #[test]
    fn task_filter_panel_separates_browsing_from_editing_and_hides_without_losing_filters() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Open task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Another task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-02".to_string()),
                        start_on: Some("2026-05-29".to_string()),
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![TaskMembershipDto {
                            project: TaskMembershipProjectDto {
                                gid: "1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        parent: None,
                        custom_fields: vec![],
                    },
                ],
            );

        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        // Selected, not just loaded: the filter view is closed off with
        // nothing selected.
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("select the project");
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
            .expect("tasks load");
        app.tasks.set_visible(true);

        let keymap = app.keymap().expect("keymap builds");

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            10,
        )
        .expect("open filter panel");
        assert!(app.tasks.filter_panel_visible());
        assert!(!app.tasks.filter_panel_editing());

        let original_kind = app
            .tasks
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.kind)
            .expect("selected filter exists");

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
            10,
        )
        .expect("cycle string mode");
        let cycled_kind = app
            .tasks
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.kind)
            .expect("selected filter exists");
        assert_ne!(original_kind, cycled_kind);

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            10,
        )
        .expect("move filter selection down");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection down");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(3)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection up");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            10,
        )
        .expect("move filter selection up");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(0)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            10,
        )
        .expect("enter edit mode");
        assert!(app.tasks.filter_panel_editing());

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('O'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.query),
            Some("Open".to_string())
        );
        assert_eq!(app.tasks.table().task_count(), 1);

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            10,
        )
        .expect("leave edit mode");
        assert!(!app.tasks.filter_panel_editing());

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            10,
        )
        .expect("hide filter panel");

        assert!(!app.tasks.filter_panel_visible());
        assert!(app.tasks.filter_summary().contains("filters 1"));
        assert_eq!(app.tasks.table().task_count(), 1);
    }

    #[test]
    fn load_projects_adds_assigned_to_me_row_only_when_the_client_resolves_a_current_user() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client.clone());
        app.load_projects().expect("projects load");
        assert_eq!(app.projects.items().len(), 1);

        let client = client.with_current_user_gid("user_1");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items()[0].id, "user_1");
        assert_eq!(app.projects.items()[0].name, "No Project (Assigned to Me)");
    }

    #[test]
    fn opening_the_assigned_to_me_row_does_not_produce_an_asana_url() {
        let client = FakeAsanaClient::new(vec![]).with_current_user_gid("user_1");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let command = app.handle_action(&Action::Open, 10).expect("open action ok");

        assert_eq!(command, None);
    }

    #[test]
    fn selecting_assigned_to_me_loads_tasks_via_the_assignee_scoped_query() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_assigned_to_me_tasks(vec![TaskDto {
                gid: "t1".to_string(),
                name: "My task".to_string(),
                completed: false,
                modified_at: None,
                due_on: None,
                start_on: None,
                assignee: None,
                num_subtasks: 0,
                memberships: vec![],
                parent: None,
                custom_fields: vec![],
            }]);

        let mut app = App::new(Config::default(), client);
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::assigned_to_me("user_1")])
            .expect("tasks load");

        assert_eq!(app.tasks.table().task_count(), 1);
    }

    /// The project list pins starred projects above the rest, so the task
    /// table's project groups have to lead with them too — otherwise the two
    /// panes disagree about the order and the eye has to hunt for the group.
    #[test]
    fn task_project_groups_follow_the_project_list_order() {
        fn task(gid: &str, name: &str, project_gid: &str, project: &str) -> TaskDto {
            TaskDto {
                gid: gid.to_string(),
                name: name.to_string(),
                completed: false,
                modified_at: None,
                due_on: None,
                start_on: None,
                assignee: None,
                num_subtasks: 0,
                memberships: vec![TaskMembershipDto {
                    project: TaskMembershipProjectDto {
                        gid: project_gid.to_string(),
                        name: project.to_string(),
                    },
                    section: None,
                }],
                parent: None,
                custom_fields: vec![],
            }
        }

        let client = FakeAsanaClient::new(vec![
            Project::new("pa", "Alpha", false),
            Project::new("pz", "Zeta", true),
        ])
        .with_tasks("pa", vec![task("t1", "Task in Alpha", "pa", "Alpha")])
        .with_tasks("pz", vec![task("t2", "Task in Zeta", "pz", "Zeta")]);

        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.handle_action(&Action::SelectAllVisible, 10)
            .expect("select every project");
        app.tasks.set_visible(true);
        app.request_task_data().expect("tasks load");
        settle(&mut app);

        let headers = app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind == crate::domain::TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            vec!["Zeta".to_string(), "Alpha".to_string()],
            "starred Zeta leads the project list, so it leads the table too"
        );
    }
}
