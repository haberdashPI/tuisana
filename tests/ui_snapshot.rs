//! Full-screen snapshots of every view at three terminal widths.
//!
//! These are the visual regression guard for the UI. They render the real
//! runtime into a `TestBackend` and compare the resulting screen against a
//! committed text file, so any change to layout, chrome, alignment, or wording
//! shows up as a readable diff rather than as a surprise on someone's terminal.
//!
//! Run `UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot` to rewrite them, then
//! read the diff before committing it.
//!
//! Determinism: `TUISANA_TODAY` pins the date the relative due dates are
//! measured against, and each fixture is loaded to completion before drawing so
//! the loading spinner never appears.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};
use std::{fs, path::PathBuf, thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto,
            SectionDto, TaskDto,
            TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto, UserDto,
        },
        fake::FakeAsanaClient,
        AsanaClient,
    },
    config::{
        Config, NamedFilterSet, ProjectVisibilityConfig, SavedFilterField, SavedFilterSet,
        ThemeConfig, ThemeGlyphs, ThemeVariant,
    },
    domain::Project,
    ui::runtime::{run_session, InputEvent, KeySource, RecordingHost},
};

const WIDTHS: [u16; 3] = [80, 120, 200];
// Tall enough that the widened fixture's twelve incomplete tasks all fit in the
// task pane. At 30 the pane clipped the last section, which hid exactly the
// rows a chart snapshot most needs to show.
const HEIGHT: u16 = 40;
const TODAY: &str = "2026-08-24";

/// Feeds a fixed script, then reports the source as closed so the session draws
/// once more and returns.
struct ScriptedSource {
    keys: Vec<KeyEvent>,
}

impl KeySource for ScriptedSource {
    fn next_event(&mut self, _timeout: Duration) -> std::io::Result<InputEvent> {
        if self.keys.is_empty() {
            return Ok(InputEvent::Closed);
        }
        Ok(InputEvent::Key(self.keys.remove(0)))
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

/// Builds one fixture task.
///
/// Positional and long, but every argument is a distinct type-free string, so
/// the order is written out here rather than inferred at each call site:
/// `gid, name, section, assignee, start, due, priority, done`.
#[allow(clippy::too_many_arguments)]
fn task(
    gid: &str,
    name: &str,
    section: &str,
    assignee: Option<&str>,
    start: Option<&str>,
    due: Option<&str>,
    priority: Option<&str>,
    done: bool,
) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: name.to_string(),
        completed: done,
        modified_at: Some("2026-08-01T00:00:00Z".to_string()),
        due_on: due.map(str::to_string),
        start_on: start.map(str::to_string),
        assignee: assignee.map(|name| UserDto {
            gid: format!("user-{name}"),
            name: Some(name.to_string()),
            display_name: Some(name.to_string()),
        }),
        num_subtasks: 0,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "project-1".to_string(),
                name: "Northwind BTX 4412".to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: section.to_string(),
                name: section.to_string(),
            }),
        }],
        parent: None,
        custom_fields: priority
            .map(|value| {
                vec![CustomFieldValueDto {
                    gid: "custom-1".to_string(),
                    name: "Priority".to_string(),
                    display_value: Some(value.to_string()),
                    enum_value: None,
                }]
            })
            .unwrap_or_default(),
    }
}

fn client() -> FakeAsanaClient {
    FakeAsanaClient::new(vec![
        Project::new("project-1", "Northwind BTX 4412", true),
        Project::new("project-2", "Platform Infrastructure", true),
        Project::new("project-3", "Backlog", false),
    ])
    .with_sections(
        "project-1",
        vec![
            SectionDto {
                gid: "Study Kit Design".to_string(),
                name: "Study Kit Design".to_string(),
            },
            SectionDto {
                gid: "Shipment".to_string(),
                name: "Shipment".to_string(),
            },
        ],
    )
    .with_custom_field_settings(
        "project-1",
        vec![ProjectCustomFieldSettingDto {
            gid: "setting-1".to_string(),
            // Declared as an enum, so the cell editor offers the options
            // the field has rather than the ones these tasks happen to hold.
            custom_field: CustomFieldDto {
                gid: "custom-1".to_string(),
                name: "Priority".to_string(),
                resource_subtype: Some("enum".to_string()),
                enum_options: vec![
                    EnumOptionDto {
                        gid: "opt-high".to_string(),
                        name: "High".to_string(),
                        enabled: true,
                    },
                    EnumOptionDto {
                        gid: "opt-low".to_string(),
                        name: "Low".to_string(),
                        enabled: true,
                    },
                ],
            },
        }],
    )
    .with_tasks(
        "project-1",
        vec![
            // Dues cover overdue, today, soon, far out, and unset, so every
            // urgency level and the empty placeholder appear in one screen.
            // Starts spread across Jun-Dec so the chart has a range worth
            // scrolling and zooming, and so bars differ in length.
            task(
                "t1",
                "Review shipment requirements doc",
                "Study Kit Design",
                Some("Morgan Ellis"),
                Some("2026-08-03"),
                Some("2026-08-19"),
                Some("High"),
                false,
            ),
            task(
                "t2",
                "Ship release candidate to the study team",
                "Study Kit Design",
                Some("Alex Chen"),
                Some("2026-08-10"),
                Some(TODAY),
                Some("High"),
                false,
            ),
            task(
                "t3",
                "Confirm carrier pickup window with the vendor",
                "Study Kit Design",
                None,
                Some("2026-08-17"),
                Some("2026-08-26"),
                None,
                false,
            ),
            task(
                "t4",
                "Close out packaging vendor contract",
                "Shipment",
                Some("Alex Chen"),
                Some("2026-10-01"),
                Some("2026-11-14"),
                Some("Low"),
                false,
            ),
            task(
                "t5",
                "Archive the pilot batch records",
                "Shipment",
                Some("Jo Park"),
                None,
                None,
                Some("Low"),
                true,
            ),
            // Eight distinct assignees across the incomplete tasks, so the
            // Gantt palette runs out and the colour dialog has to show a
            // neutral group. Six get a colour; the rest do not.
            task(
                "t6",
                "Draft the assay validation protocol",
                "Study Kit Design",
                Some("Priya Raman"),
                Some("2026-06-15"),
                Some("2026-07-10"),
                Some("High"),
                false,
            ),
            task(
                "t7",
                "Qualify the second reagent supplier",
                "Study Kit Design",
                Some("Dana Ruiz"),
                Some("2026-07-01"),
                Some("2026-09-12"),
                Some("Medium"),
                false,
            ),
            task(
                "t8",
                "Update the sample intake SOP",
                "Study Kit Design",
                Some("Kim Alvarez"),
                Some("2026-08-20"),
                Some("2026-09-04"),
                Some("Medium"),
                false,
            ),
            task(
                "t9",
                "Book the courier for the pilot run",
                "Shipment",
                Some("Sam Okafor"),
                Some("2026-09-01"),
                Some("2026-09-15"),
                Some("Low"),
                false,
            ),
            task(
                "t10",
                "Reconcile the freight invoices",
                "Shipment",
                Some("Robin Fox"),
                Some("2026-09-20"),
                Some("2026-10-30"),
                Some("Low"),
                false,
            ),
            // A due date with no start: the chart draws this as a milestone
            // rather than a bar.
            task(
                "t11",
                "Label the retention samples",
                "Shipment",
                Some("Jo Park"),
                None,
                Some("2026-10-09"),
                Some("Medium"),
                false,
            ),
            task(
                "t12",
                "Close the quarter's shipping report",
                "Shipment",
                Some("Priya Raman"),
                Some("2026-11-02"),
                Some("2026-12-18"),
                Some("Low"),
                false,
            ),
        ],
    )
}

/// The monochrome, ASCII-only fallback: no color codes and no glyphs beyond
/// plain ASCII, for terminals that cannot render either.
fn mono_config() -> Config {
    let mut config = config();
    config.theme = ThemeConfig {
        variant: ThemeVariant::Mono,
        glyphs: ThemeGlyphs::Ascii,
        accent: "cyan".to_string(),
        zebra: false,
    };
    config
}

fn config() -> Config {
    // Without visibility entries every project defaults to hidden, which is a
    // real state but an unrepresentative one to snapshot.
    let mut config = Config::default();
    config.project_visibility = vec![
        ProjectVisibilityConfig {
            gid: "project-1".to_string(),
            starred: true,
            hidden: false,
        },
        ProjectVisibilityConfig {
            gid: "project-2".to_string(),
            starred: true,
            hidden: false,
        },
        ProjectVisibilityConfig {
            gid: "project-3".to_string(),
            starred: false,
            hidden: false,
        },
    ];
    config
}

fn drain_task_data<C: AsanaClient + Clone + Send + 'static>(app: &mut App<C>) {
    for _ in 0..100 {
        app.poll_task_data();
        if !matches!(app.tasks.status(), tuisana::app::task::TaskStatus::Loading) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Renders one screen from a sequence of key batches.
///
/// Task loading is asynchronous, so each batch is followed by a drain: keys in
/// a later batch always act on a settled table. Sending everything at once
/// makes the result depend on whether the worker thread happened to finish
/// first, which is exactly the flakiness a snapshot test must not have.
fn render_with(config: Config, width: u16, batches: Vec<Vec<KeyEvent>>) -> String {
    let buffer = render_buffer(config, width, batches);
    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Draws one screen and hands back the styled buffer behind it.
///
/// Split out of `render_with` because the screenshot the README carries needs
/// the colors, and the text snapshots deliberately throw them away.
fn render_buffer(config: Config, width: u16, batches: Vec<Vec<KeyEvent>>) -> Buffer {
    let mut app = App::new(config, client());
    app.load_projects().expect("projects load");

    let backend = TestBackend::new(width, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("terminal");

    for keys in batches {
        run_session(&mut app, &mut ScriptedSource { keys }, &mut terminal, &mut RecordingHost::default())
            .expect("session runs");
        drain_task_data(&mut app);
    }

    // A final pass with no input redraws the settled state, so a snapshot never
    // captures a spinner mid-load.
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal, &mut RecordingHost::default())
        .expect("session redraws");

    terminal.backend_mut().buffer().clone()
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"))
}

fn assert_snapshot(name: &str, batches: fn() -> Vec<Vec<KeyEvent>>) {
    assert_snapshot_with(config(), WIDTHS.to_vec(), name, batches)
}

fn assert_snapshot_with(
    config: Config,
    widths: Vec<u16>,
    name: &str,
    batches: fn() -> Vec<Vec<KeyEvent>>,
) {
    std::env::set_var("TUISANA_TODAY", TODAY);

    for width in widths {
        let rendered = render_with(config.clone(), width, batches());
        let path = snapshot_path(&format!("{name}-{width}"));

        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            fs::create_dir_all(path.parent().expect("snapshot dir")).expect("create snapshot dir");
            fs::write(&path, format!("{rendered}\n")).expect("write snapshot");
            continue;
        }

        let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot",
                path.display()
            )
        });

        assert_eq!(
            rendered,
            expected.trim_end_matches('\n'),
            "snapshot {name}-{width} changed; \
             run UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot to accept"
        );
    }
}

/// Select the first project and open the task view, leaving it fully loaded.
fn enter_task_mode() -> Vec<KeyEvent> {
    vec![key(' '), key('t')]
}

#[test]
fn project_mode() {
    assert_snapshot("project-mode", Vec::new);
}

#[test]
fn project_mode_with_help() {
    assert_snapshot("project-help", || vec![vec![key('?')]]);
}

#[test]
fn project_search() {
    assert_snapshot("project-search", || vec![vec![key('/'), key('l')]]);
}

#[test]
fn gantt_mode() {
    assert_snapshot("gantt", || vec![vec![key(' '), key('t')], vec![key('g')]]);
}

/// Zoomed in twice and scrolled, so bars run past the window and the status
/// bar has to say where the window is.
#[test]
fn gantt_zoomed_and_scrolled() {
    assert_snapshot("gantt-zoom", || {
        vec![
            vec![key(' '), key('t')],
            vec![key('g'), key('='), key('='), key('l')],
        ]
    });
}

/// More table columns, taking the space from the chart.
#[test]
fn gantt_with_more_columns() {
    assert_snapshot("gantt-columns", || {
        vec![vec![key(' '), key('t')], vec![key('g'), key('>'), key('>')]]
    });
}

/// Zoomed until a month fits the pane: the axis marks weeks, not months.
#[test]
fn gantt_zoomed_to_weeks() {
    assert_snapshot_with(config(), vec![120], "gantt-weeks", || {
        vec![
            vec![key(' '), key('t')],
            vec![key('g'), key('='), key('='), key('='), key('=')],
        ]
    });
}

/// Zoomed to a fortnight: the axis marks individual days, and the weekend
/// columns are shaded.
#[test]
fn gantt_zoomed_to_days() {
    assert_snapshot_with(config(), vec![120], "gantt-days", || {
        vec![
            vec![key(' '), key('t')],
            vec![key('g'), key('='), key('='), key('='), key('='), key('=')],
        ]
    });
}

/// Zoomed to a week and centred on today, so the axis names weekdays and
/// bars actually run across the shaded weekend.
#[test]
fn gantt_zoomed_to_weekdays() {
    assert_snapshot_with(config(), vec![120], "gantt-weekdays", || {
        vec![
            vec![key(' '), key('t')],
            vec![
                key('g'),
                key('='),
                key('='),
                key('='),
                key('='),
                key('='),
                key('='),
                key('t'),
            ],
        ]
    });
}

/// The help overlay for gantt mode, which is where someone looks for the
/// zoom keys.
#[test]
fn gantt_mode_with_help() {
    assert_snapshot_with(config(), vec![120], "gantt-help", || {
        vec![vec![key(' '), key('t')], vec![key('g'), key('?')]]
    });
}

/// The colour dialog, with more than six values so the palette rule shows.
#[test]
fn gantt_color_dialog() {
    assert_snapshot("gantt-order", || {
        vec![vec![key(' '), key('t')], vec![key('g'), enter()]]
    });
}

/// The monochrome ASCII fallback: no colour, so the six palette slots have to
/// be told apart by the bar texture alone.
#[test]
fn gantt_in_the_monochrome_ascii_theme() {
    assert_snapshot_with(mono_config(), vec![120], "gantt-mono", || {
        vec![vec![key(' '), key('t')], vec![key('g')]]
    });
}

/// Six table columns at 80: the columns region hits its share cap, the
/// clipped columns stay reachable by scrolling, and the legend runs out of
/// border and says how much it dropped.
///
/// The chart being dropped entirely needs a terminal under about 32 columns;
/// `a_pane_too_narrow_for_a_chart_says_so_instead_of_drawing_one` covers that.
#[test]
fn gantt_with_every_column_at_eighty() {
    assert_snapshot_with(config(), vec![80], "gantt-many-columns", || {
        vec![
            vec![key(' '), key('t')],
            vec![key('g'), key('>'), key('>'), key('>'), key('>')],
        ]
    });
}

/// Coloured by section rather than assignee.
#[test]
fn gantt_coloured_by_section() {
    assert_snapshot("gantt-color-section", || {
        vec![vec![key(' '), key('t')], vec![key('g'), key('c')]]
    });
}

#[test]
fn task_mode() {
    assert_snapshot("task-mode", || vec![enter_task_mode()]);
}

#[test]
fn task_mode_with_selection() {
    assert_snapshot("task-selection", || {
        vec![enter_task_mode(), vec![key(' '), key(' ')]]
    });
}

/// The window that asks before a version-1 config is rewritten.
///
/// It is the first thing a user upgrading sees, and the one window that must
/// name the file it is about to write. Written into a directory of its own so
/// the config is called `tuisana.toml` and the backup name is the real one.
#[test]
fn the_version_one_migration_prompt() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("tuisana-snapshot-{unique}"));
    fs::create_dir_all(&directory).expect("create the config directory");
    let path = directory.join("tuisana.toml");
    fs::write(&path, "[header]\ntype = \"tuisana\"\nversion = 1.0\n")
        .expect("write a version 1 config");

    let mut config = Config::load_from_path(&path).expect("config loads");
    config.project_visibility = self::config().project_visibility;
    assert_snapshot_with(config, WIDTHS.to_vec(), "config-migration", Vec::new);

    fs::remove_dir_all(&directory).ok();
}

/// Edit mode: a different badge, a different hint bar, the same rows.
///
/// The split is the point — task mode's keys are about what the table *says*,
/// these are about what it *contains* — and the two bars are what say so.
#[test]
fn edit_mode() {
    assert_snapshot("edit-mode", || vec![enter_task_mode(), vec![key('t')]]);
}

/// Three rows marked for deletion, struck through, with `deleting 3` on the
/// border.
///
/// A glyph change rather than a colour change, so it reads under
/// `variant = "mono"` too.
#[test]
fn edit_mode_with_rows_marked_for_deletion() {
    assert_snapshot("edit-marked", || {
        vec![
            enter_task_mode(),
            // `space` selects and advances, so three presses take three rows;
            // `x` marks the whole selection at once.
            vec![key('t'), key(' '), key(' '), key(' '), key('x')],
        ]
    });
}

/// A draft task on screen, directly after the cursor row, with its title
/// half-typed.
#[test]
fn edit_mode_with_a_draft_task() {
    assert_snapshot("edit-draft", || {
        let mut keys = vec![key('t'), key('i')];
        keys.extend("Pack the retention samples".chars().map(key));
        vec![enter_task_mode(), keys]
    });
}

/// A draft section, which is a heading being typed rather than a cell.
#[test]
fn edit_mode_with_a_draft_section() {
    assert_snapshot("edit-draft-section", || {
        let mut keys = vec![key('t'), KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)];
        keys.extend("Retention".chars().map(key));
        vec![enter_task_mode(), keys]
    });
}

/// The help overlay for edit mode, which is the short honest list the mode
/// split buys.
#[test]
fn edit_mode_with_help() {
    assert_snapshot("edit-help", || {
        vec![enter_task_mode(), vec![key('t'), key('?')]]
    });
}

#[test]
fn task_mode_with_help() {
    assert_snapshot("task-help", || vec![enter_task_mode(), vec![key('?')]]);
}

#[test]
fn task_mode_showing_completed_tasks() {
    assert_snapshot("task-all-states", || {
        vec![enter_task_mode(), vec![key('c')], vec![key('c')]]
    });
}

/// The column cursor on Due, with nothing being edited.
///
/// Pins the header accent and the cursor row's underline — and, by staying
/// unchanged, that the Gantt snapshots never grew a cursor they cannot move.
#[test]
fn task_mode_with_the_column_cursor_moved() {
    assert_snapshot("task-column-cursor", || {
        vec![enter_task_mode(), vec![key('l'), key('l')]]
    });
}

/// Two columns sorted, Assignee toggled after Due.
///
/// Pins the ranked header marks — the arrow alone cannot say which of two
/// sorted columns the rows are ordered by — and the chip that spells the same
/// thing out in words.
#[test]
fn task_mode_sorted_by_two_columns() {
    assert_snapshot("task-sorted-columns", || {
        vec![
            enter_task_mode(),
            vec![key('l'), key('l'), key('s')],
            vec![key('h'), key('s')],
        ]
    });
}

/// Mid-edit on a title longer than its column, caret at the end.
#[test]
fn task_mode_editing_a_long_title() {
    assert_snapshot("task-edit-title", || {
        let mut keys = vec![enter()];
        keys.extend(" and every word after the column runs out".chars().map(key));
        vec![enter_task_mode(), keys]
    });
}

/// The Priority picker open with two tasks selected.
///
/// Pins the option display and the `editing 2` chip, which is the only thing
/// on screen that says how wide the commit reaches.
#[test]
fn task_mode_editing_a_value_picker() {
    assert_snapshot("task-edit-options", || {
        vec![
            enter_task_mode(),
            vec![key(' '), key(' ')],
            vec![key('l'), key('l'), key('l'), key('l'), key('l'), key('l')],
            vec![enter(), key('j')],
        ]
    });
}

/// The assignee cell mid-completion, with the candidate list over it.
///
/// Pins the overlay's position and the highlighted row `tab` landed on — the
/// cell is far too narrow to list names in, so this is the only place the
/// choices are visible.
#[test]
fn task_mode_completing_an_assignee() {
    assert_snapshot("task-edit-assignee", || {
        vec![
            enter_task_mode(),
            vec![key('l'), enter(), ctrl('l')],
            vec![key('a'), tab()],
        ]
    });
}

/// A task marked done while the table is showing only open ones, so the row
/// it was on has left the table and the pane below the project list is
/// holding it.
/// The corner notice, over the task pane it used to be a border chip on.
///
/// A refusal rather than a failed write: the fake backend here accepts
/// everything, and what the snapshot is guarding is the box, not the message.
#[test]
fn task_mode_with_a_notice() {
    assert_snapshot("task-notice", || {
        vec![
            enter_task_mode(),
            // Two rows selected, then `enter` on the title: a title is edited
            // one task at a time, which is refused rather than done.
            vec![key(' '), key(' '), enter()],
        ]
    });
}

#[test]
fn task_mode_with_a_recently_edited_task() {
    assert_snapshot("task-recent-edits", || {
        vec![enter_task_mode(), vec![key('d')]]
    });
}

/// The `Assignee` filter row in `list` mode, picking from the same directory
/// the cell editor offers.
#[test]
fn filter_mode_picking_from_a_list() {
    assert_snapshot("filter-list-mode", || {
        vec![
            enter_task_mode(),
            vec![key('f'), key('j')],
            vec![key('s'), key('s'), key('s')],
            vec![enter()],
        ]
    });
}

#[test]
fn filter_mode() {
    assert_snapshot("filter-mode", || vec![enter_task_mode(), vec![key('f')]]);
}

/// The panel stays up top when the keys go back to the table: a thin border
/// on the filters, a thick one on the table.
#[test]
fn task_mode_with_the_filter_panel_still_open() {
    assert_snapshot("task-with-filters", || {
        vec![enter_task_mode(), vec![key('f')], vec![key('t')]]
    });
}

#[test]
fn filter_edit_mode() {
    assert_snapshot("filter-edit", || {
        vec![
            enter_task_mode(),
            vec![key('f'), enter()],
            vec![key('s'), key('h')],
        ]
    });
}

/// Two sets, the second active and both filtering, so the border-mounted tab
/// strip, its `any of` lead-in, and the marker on the set left behind are all
/// on screen.
#[test]
fn filter_mode_with_two_sets() {
    assert_snapshot("filter-sets", || {
        vec![
            enter_task_mode(),
            // `j` to Assignee, `enter` to edit it — in filter *browse* mode the
            // letters are bound (`h`/`l` switch sets, `a` adds one), so the
            // value has to be typed from edit mode.
            vec![key('f'), key('j'), enter()],
            "alex".chars().map(key).chain([enter()]).collect(),
            // `a` adds a set. The field cursor is shared, so one `j` moves both
            // sets' cursor from Assignee to Due; `e` requires it to be empty.
            vec![key('a'), key('j'), key('e')],
        ]
    });
}

/// Both negations at once: a negated `Assignee` row inside a negated set, so
/// the row's `¬`, the tab's `¬` and its heavier edges, and the `negated set`
/// chip are all on one frame.
#[test]
fn filter_mode_with_negations() {
    assert_snapshot("filter-negated", || {
        vec![
            enter_task_mode(),
            vec![key('f'), key('j'), enter()],
            "alex".chars().map(key).chain([enter()]).collect(),
            // `!` inverts the row the cursor is still on and `~` the set
            // around it. `a` then `h` adds a plain second set and steps back,
            // because a lone set draws no strip to mark.
            vec![key('!'), key('~'), key('a'), key('h')],
        ]
    });
}

/// A config carrying four saved filter sets, for the sidebar scenarios.
///
/// Written into the config the harness builds rather than typed at the
/// prompt: four `w` cycles would be forty keystrokes of setup for a picture
/// of the result.
fn named_sets_config() -> Config {
    fn entry(name: &str, key: &str, query: &str) -> NamedFilterSet {
        NamedFilterSet {
            name: name.to_string(),
            scratch: false,
            projects: None,
            sets: vec![SavedFilterSet {
                negated: false,
                fields: vec![SavedFilterField {
                    key: key.to_string(),
                    query: query.to_string(),
                    ..SavedFilterField::default()
                }],
            }],
        }
    }

    let mut config = config();
    config.filter_sets = vec![
        entry("Blocked", "custom:Priority", "High"),
        entry("Overdue mine", "assignee", "alex"),
        entry("Sprint triage", "title", "ship"),
        entry("Waiting on", "assignee", "jo"),
    ];
    // One entry with a `projects` list and three without, so both halves of
    // the absent/empty split are on screen somewhere: the confirmation's
    // project lines appear for `Blocked` and the sidebar's project count is
    // the live selection either way.
    config.filter_sets[0].projects =
        Some(vec!["project-2".to_string(), "project-3".to_string()]);
    config
}

/// Four saved entries with the third loaded and the sidebar open, so the
/// `current` row, the rule, the numbering, the loaded marker, and the sidebar
/// giving way at 80 columns are all pinned.
#[test]
fn filter_mode_with_the_named_set_sidebar() {
    assert_snapshot_with(named_sets_config(), WIDTHS.to_vec(), "filter-sets-named", || {
        vec![enter_task_mode(), vec![key('f'), key('b'), key('3')]]
    });
}

/// The project view with the sidebar up, so the four-line header, its project
/// count, the bound-set chip on the project pane's border, and the sidebar
/// surviving at eighty columns — where the filter view drops it — are pinned.
#[test]
fn project_mode_with_the_named_set_sidebar() {
    assert_snapshot_with(
        named_sets_config(),
        WIDTHS.to_vec(),
        "project-sets-named",
        || vec![vec![key(' '), key('b'), key('1')]],
    );
}

/// `w` from the project view: the prompt on the sidebar's border, with the
/// project list rather than the filter rows beside it.
#[test]
fn project_mode_naming_a_set() {
    assert_snapshot_with(
        named_sets_config(),
        vec![120],
        "project-sets-save-prompt",
        || {
            vec![
                vec![key(' '), key('b'), key('w')],
                "sprint".chars().map(key).collect(),
            ]
        },
    );
}

/// `w` in the project view on a terminal too narrow for the sidebar: the
/// prompt borrows the project pane's bottom border, where it displaces the
/// search footer rather than sharing the line.
#[test]
fn project_mode_naming_a_set_without_room_for_the_sidebar() {
    assert_snapshot_with(
        named_sets_config(),
        vec![40],
        "project-sets-save-prompt-narrow",
        || {
            vec![
                vec![key('/')],
                "back".chars().map(key).collect(),
                vec![enter(), key('w')],
                "sprint".chars().map(key).collect(),
            ]
        },
    );
}

/// Mid-`w`, with text typed, so the border-mounted prompt is pinned.
#[test]
fn filter_mode_naming_a_set() {
    assert_snapshot_with(
        named_sets_config(),
        WIDTHS.to_vec(),
        "filter-sets-save-prompt",
        || {
            vec![
                enter_task_mode(),
                vec![key('f'), key('b'), key('w')],
                "sprint".chars().map(key).collect(),
            ]
        },
    );
}

/// `d` over a selection past the threshold, so the question a bulk edit asks
/// — and the count in its title — are pinned.
///
/// The column band that goes with it cannot be snapshotted: these capture
/// symbols, not styles. `ui::task_table::tests` covers that half.
#[test]
fn task_mode_confirming_a_bulk_edit() {
    let mut config = config();
    // Two rows is already "more than a few" here, which keeps the fixture
    // small enough to read.
    config.edit.confirm_threshold = 1;

    assert_snapshot_with(config, WIDTHS.to_vec(), "bulk-edit-confirm", || {
        vec![
            enter_task_mode(),
            // `space` selects and moves down, so the cursor ends on a third
            // row that is not among the targets.
            vec![key(' '), key(' '), key('d')],
        ]
    });
}

/// A digit pressed over an unnamed panel that is filtering, so the
/// border-mounted confirmation and the `y`/`n` hints are pinned.
#[test]
fn filter_mode_confirming_a_load_over_unsaved_filters() {
    assert_snapshot_with(
        named_sets_config(),
        WIDTHS.to_vec(),
        "filter-sets-load-confirm",
        || {
            vec![
                enter_task_mode(),
                // A filter typed into the unnamed panel the app starts with,
                // then `1` — which would throw it away.
                vec![key('f'), key('b'), key('j'), enter()],
                "alex".chars().map(key).chain([enter()]).collect(),
                vec![key('1')],
            ]
        },
    );
}

/// `d` over a loaded entry: the other confirmation, which says what leaving
/// and what staying actually mean.
#[test]
fn filter_mode_confirming_a_delete() {
    assert_snapshot_with(
        named_sets_config(),
        vec![120],
        "filter-sets-delete-confirm",
        || {
            vec![
                enter_task_mode(),
                vec![key('f'), key('b'), key('3'), key('d')],
            ]
        },
    );
}

/// A require-empty on `Due`, so `(none)` is drawn next to the `—` of the
/// untouched rows and the two are visibly different.
#[test]
fn filter_mode_requiring_an_empty_due_date() {
    assert_snapshot("filter-require-empty", || {
        vec![enter_task_mode(), vec![key('f'), key('j'), key('j'), key('e')]]
    });
}

/// The `Due` filter is the third row, so two `j`s land on it.
fn open_due_calendar() -> Vec<KeyEvent> {
    vec![key('f'), key('j'), key('j'), enter()]
}

#[test]
fn filter_date_calendar() {
    assert_snapshot("filter-date-calendar", || {
        vec![enter_task_mode(), open_due_calendar()]
    });
}

#[test]
fn filter_date_calendar_after_flipping_two_months_forward() {
    assert_snapshot("filter-date-calendar-month", || {
        vec![enter_task_mode(), open_due_calendar(), vec![key('j'), key('j')]]
    });
}

#[test]
fn filter_date_calendar_with_unusable_text() {
    // Letters only reach the text with the grid put away, so `;` either side
    // of them is what gets `abc` typed at all. The overlay has to say the text
    // is unusable, because the old behavior was to silently filter the table
    // to nothing.
    assert_snapshot("filter-date-calendar-invalid", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            vec![key(';'), key('a'), key('b'), key('c'), key(';')],
        ]
    });
}

#[test]
fn filter_date_calendar_with_a_range() {
    // Digits, `-`, and `.` are unbound in calendar mode, so they type into the
    // filter field. The grid shades the span between the two ends.
    assert_snapshot("filter-date-calendar-range", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            "2026-08-10..2026-08-20".chars().map(key).collect(),
        ]
    });
}

#[test]
fn filter_date_calendar_editing_the_start_of_a_range() {
    // `ctrl-a` moves the caret to the start end, so the navigation keys rewrite
    // that end and the overlay says which one it is editing.
    assert_snapshot("filter-date-calendar-range-start", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            "2026-08-10..2026-08-20".chars().map(key).collect(),
            vec![ctrl('a'), key('h')],
        ]
    });
}

#[test]
fn filter_date_calendar_shading_a_whole_month_from_a_keyword() {
    // `this month` names the month, not its first day, so the grid shades all
    // of it — the same shading a written `2026-08-01..2026-08-31` earns.
    assert_snapshot("filter-date-calendar-this-month", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            vec![key(';')],
            "this month".chars().map(key).collect(),
            vec![key(';')],
        ]
    });
}

#[test]
fn filter_date_calendar_with_the_grid_hidden() {
    // `;` puts the grid away and hands its letters back to the text, which is
    // the only way `tue` can be typed: `t` is `today` while the grid is up.
    // What is left is the resolved value, so the keyword is still legible.
    assert_snapshot("filter-date-calendar-typed", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            vec![key(';')],
            "tue".chars().map(key).collect(),
        ]
    });
}

#[test]
fn filter_date_calendar_help() {
    // Calendar mode does not fall back to global bindings, so `?` has to be
    // bound there for the picker's own keys to be discoverable.
    assert_snapshot("filter-date-calendar-help", || {
        vec![enter_task_mode(), open_due_calendar(), vec![key('?')]]
    });
}

#[test]
fn task_mode_in_the_monochrome_ascii_theme() {
    assert_snapshot_with(mono_config(), vec![120], "task-mono", || {
        vec![enter_task_mode()]
    });
}

#[test]
fn the_monochrome_theme_emits_no_color() {
    // The buffer is inspected directly here rather than snapshotted, because a
    // text snapshot cannot show whether a style was applied.
    std::env::set_var("TUISANA_TODAY", TODAY);
    let mut app = App::new(mono_config(), client());
    app.load_projects().expect("projects load");

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    run_session(
        &mut app,
        &mut ScriptedSource {
            keys: enter_task_mode(),
        },
        &mut terminal,
        &mut RecordingHost::default(),
    )
    .expect("session runs");
    drain_task_data(&mut app);
    // The calendar overlay is checked in the same pass: it draws day numbers, a
    // highlight, and a shaded range, all of which are easy to reach for color or
    // a box-drawing glyph without noticing.
    // `a` adds a second filter set before the picker opens, so the tab strip
    // and its active marker are inside this pass rather than beside it, and
    // `~` then `!` bring both negation markers — the tab edge and the row
    // glyph — along with it. The keys are spelled out rather than reusing
    // `open_due_calendar`, whose leading `f` would close the panel `a` needs
    // open.
    let mut keys = vec![
        key('f'),
        key('a'),
        key('~'),
        key('j'),
        key('j'),
        key('!'),
        enter(),
    ];
    keys.extend("2026-08-10..2026-08-20".chars().map(key));
    run_session(
        &mut app,
        &mut ScriptedSource { keys },
        &mut terminal,
        &mut RecordingHost::default(),
    )
    .expect("session opens the calendar");
    drain_task_data(&mut app);
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal, &mut RecordingHost::default())
        .expect("session redraws");

    assert!(
        app.tasks.calendar_open(),
        "the calendar is what this pass is checking"
    );
    assert!(
        app.tasks.filter_active_set_negated(),
        "and a negated set is what puts the ascii tab edge on screen"
    );

    let buffer = terminal.backend_mut().buffer().clone();
    for cell in buffer.content.iter() {
        assert_eq!(
            cell.fg,
            ratatui::style::Color::Reset,
            "the monochrome theme set a foreground color"
        );
        assert_eq!(
            cell.bg,
            ratatui::style::Color::Reset,
            "the monochrome theme set a background color"
        );
        assert!(
            cell.symbol().is_ascii(),
            "the ascii glyph set emitted {:?}",
            cell.symbol()
        );
    }
}

#[test]
fn the_example_config_parses() {
    let contents = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tuisana.toml.example"),
    )
    .expect("read the example config");

    Config::from_toml_str(&contents).expect("the documented example config is valid");
}

#[test]
fn task_pane_with_the_project_pane_minimized() {
    // `{` minimizes the top pane and switches to task mode, so the task table
    // gets the whole body.
    assert_snapshot("task-minimized-top", || vec![vec![key(' '), key('{')]]);
}

// ---------------------------------------------------------------------------
// The README's screenshot
// ---------------------------------------------------------------------------
//
// The picture at the top of the README is generated from the same fake client
// and the same fixture as the snapshots above, so it can never drift from what
// the app actually draws, and it never shows anyone's real Asana data. It is
// written as an SVG of the terminal grid rather than captured from a real
// terminal: every cell is placed at an explicit coordinate, so the image lines
// up whatever monospace font the reader's browser happens to have.
//
// Regenerate with:
//
//     cargo test --test ui_snapshot readme_screenshot -- --ignored

/// Width of one terminal cell, in pixels, and the font size that fills it.
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
const FONT_SIZE: f32 = 14.0;
/// Distance from the top of a row to the text baseline.
const BASELINE: f32 = 13.5;
const PAD: f32 = 14.0;
const FONT_STACK: &str =
    "ui-monospace,'SF Mono',Menlo,Consolas,'DejaVu Sans Mono','Liberation Mono',monospace";

const BACKGROUND: &str = "#15181e";
const FOREGROUND: &str = "#c6ccd6";

/// The sixteen ANSI colors, in a dark-terminal palette.
const ANSI: [&str; 16] = [
    "#1b1f27", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
    "#5c6370", "#ff7b86", "#b5e890", "#ffd38a", "#8ccbff", "#e39df5", "#7fd6e0", "#e6e6e6",
];

/// Resolves one xterm palette index to a hex color.
fn indexed_hex(index: u8) -> String {
    match index {
        0..=15 => ANSI[index as usize].to_string(),
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| match v {
                0 => 0u32,
                other => 55 + 40 * other as u32,
            };
            format!(
                "#{:02x}{:02x}{:02x}",
                level(i / 36),
                level((i / 6) % 6),
                level(i % 6)
            )
        }
        _ => {
            let shade = 8 + 10 * (index as u32 - 232);
            format!("#{shade:02x}{shade:02x}{shade:02x}")
        }
    }
}

/// Resolves a ratatui color, falling back to the terminal default the cell
/// would have inherited.
fn color_hex(color: Color, default: &str) -> String {
    match color {
        Color::Reset => default.to_string(),
        Color::Black => ANSI[0].to_string(),
        Color::Red => ANSI[1].to_string(),
        Color::Green => ANSI[2].to_string(),
        Color::Yellow => ANSI[3].to_string(),
        Color::Blue => ANSI[4].to_string(),
        Color::Magenta => ANSI[5].to_string(),
        Color::Cyan => ANSI[6].to_string(),
        Color::Gray => ANSI[7].to_string(),
        Color::DarkGray => ANSI[8].to_string(),
        Color::LightRed => ANSI[9].to_string(),
        Color::LightGreen => ANSI[10].to_string(),
        Color::LightYellow => ANSI[11].to_string(),
        Color::LightBlue => ANSI[12].to_string(),
        Color::LightMagenta => ANSI[13].to_string(),
        Color::LightCyan => ANSI[14].to_string(),
        Color::White => ANSI[15].to_string(),
        Color::Indexed(index) => indexed_hex(index),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

fn escape(symbol: &str) -> String {
    symbol
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// How one cell is painted, once reverse video has been resolved away.
#[derive(PartialEq)]
struct CellPaint {
    fg: String,
    bg: String,
    bold: bool,
    dim: bool,
    italic: bool,
    underlined: bool,
}

fn paint(cell: &ratatui::buffer::Cell) -> CellPaint {
    let reversed = cell.modifier.contains(Modifier::REVERSED);
    let (fg_color, bg_color) = match reversed {
        true => (cell.bg, cell.fg),
        false => (cell.fg, cell.bg),
    };
    let (fg_default, bg_default) = match reversed {
        true => (BACKGROUND, FOREGROUND),
        false => (FOREGROUND, BACKGROUND),
    };

    CellPaint {
        fg: color_hex(fg_color, fg_default),
        bg: color_hex(bg_color, bg_default),
        bold: cell.modifier.contains(Modifier::BOLD),
        dim: cell.modifier.contains(Modifier::DIM),
        italic: cell.modifier.contains(Modifier::ITALIC),
        underlined: cell.modifier.contains(Modifier::UNDERLINED),
    }
}

/// Draws the buffer as an SVG: one rect per run of background, one text run per
/// run of identical styling, with every glyph given its own x coordinate.
fn buffer_to_svg(buffer: &Buffer) -> String {
    let width = buffer.area.width as f32 * CELL_W + 2.0 * PAD;
    let height = buffer.area.height as f32 * CELL_H + 2.0 * PAD;

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"{FONT_STACK}\" \
         font-size=\"{FONT_SIZE}\">\n\
         <rect width=\"{width:.0}\" height=\"{height:.0}\" rx=\"10\" fill=\"{BACKGROUND}\"/>\n"
    );

    for row in 0..buffer.area.height {
        let y = PAD + row as f32 * CELL_H;
        let cells: Vec<_> = (0..buffer.area.width)
            .map(|column| buffer.cell((column, row)).expect("cell in buffer"))
            .collect();
        let paints: Vec<_> = cells.iter().map(|cell| paint(cell)).collect();

        // Backgrounds first, as whole runs, so neighbouring banded cells make
        // one unbroken band rather than a row of abutting rectangles.
        let mut start = 0usize;
        while start < paints.len() {
            let mut end = start + 1;
            while end < paints.len() && paints[end].bg == paints[start].bg {
                end += 1;
            }
            if paints[start].bg != BACKGROUND {
                let x = PAD + start as f32 * CELL_W;
                let run = (end - start) as f32 * CELL_W;
                svg.push_str(&format!(
                    "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{run:.1}\" height=\"{CELL_H:.1}\" \
                     fill=\"{}\"/>\n",
                    paints[start].bg
                ));
            }
            start = end;
        }

        let mut start = 0usize;
        while start < paints.len() {
            let mut end = start + 1;
            while end < paints.len() && paints[end] == paints[start] {
                end += 1;
            }

            let text: String = cells[start..end]
                .iter()
                .map(|cell| escape(cell.symbol()))
                .collect();
            if !text.trim().is_empty() {
                let xs: Vec<String> = (start..end)
                    .filter(|column| !cells[*column].symbol().is_empty())
                    .map(|column| format!("{:.1}", PAD + column as f32 * CELL_W))
                    .collect();
                let style = &paints[start];
                let mut attributes = format!(
                    "x=\"{}\" y=\"{:.1}\" fill=\"{}\"",
                    xs.join(" "),
                    y + BASELINE,
                    style.fg
                );
                if style.bold {
                    attributes.push_str(" font-weight=\"bold\"");
                }
                if style.italic {
                    attributes.push_str(" font-style=\"italic\"");
                }
                if style.underlined {
                    attributes.push_str(" text-decoration=\"underline\"");
                }
                if style.dim {
                    attributes.push_str(" opacity=\"0.6\"");
                }
                svg.push_str(&format!("<text {attributes}>{text}</text>\n"));
            }

            start = end;
        }
    }

    svg.push_str("</svg>\n");
    svg
}

/// Writes the README's screenshot: the Gantt view, which is the one frame that
/// shows the project list, the task table, and the chart at once.
#[test]
#[ignore = "writes docs/public/screenshot.svg rather than asserting anything"]
fn readme_screenshot() {
    std::env::set_var("TUISANA_TODAY", TODAY);

    let buffer = render_buffer(
        config(),
        120,
        vec![vec![key(' '), key('t')], vec![key('g')]],
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/public/screenshot.svg");
    fs::create_dir_all(path.parent().expect("screenshot dir")).expect("create screenshot dir");
    fs::write(&path, buffer_to_svg(&buffer)).expect("write screenshot");
}
