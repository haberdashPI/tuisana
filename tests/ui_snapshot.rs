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
use ratatui::{backend::TestBackend, Terminal};
use std::{fs, path::PathBuf, thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto, TaskDto,
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
    ui::runtime::{run_session, InputEvent, KeySource},
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
            custom_field: CustomFieldDto {
                gid: "custom-1".to_string(),
                name: "Priority".to_string(),
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
    let mut app = App::new(config, client());
    app.load_projects().expect("projects load");

    let backend = TestBackend::new(width, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("terminal");

    for keys in batches {
        run_session(&mut app, &mut ScriptedSource { keys }, &mut terminal)
            .expect("session runs");
        drain_task_data(&mut app);
    }

    // A final pass with no input redraws the settled state, so a snapshot never
    // captures a spinner mid-load.
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal)
        .expect("session redraws");

    let buffer = terminal.backend_mut().buffer().clone();
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

#[test]
fn filter_mode() {
    assert_snapshot("filter-mode", || vec![enter_task_mode(), vec![key('f')]]);
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
    // `a`, `b`, and `c` are unbound in calendar mode, so they type rather than
    // navigating. The overlay has to say the text is unusable, because the old
    // behavior was to silently filter the table to nothing.
    assert_snapshot("filter-date-calendar-invalid", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            vec![key('a'), key('b'), key('c')],
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
    )
    .expect("session opens the calendar");
    drain_task_data(&mut app);
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal)
        .expect("session redraws");

    assert!(
        app.tasks.filter_calendar_open(),
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
