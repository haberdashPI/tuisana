use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use tuisana::{
    app::App,
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto, TaskDto,
            TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto, UserDto,
        },
        fake::FakeAsanaClient,
    },
    config::Config,
    domain::Project,
    ui::runtime::{run_project_list_session, KeySource},
};

struct ScriptedSource {
    keys: Vec<KeyEvent>,
}

impl KeySource for ScriptedSource {
    fn next_key(&mut self) -> std::io::Result<Option<KeyEvent>> {
        if self.keys.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.keys.remove(0)))
        }
    }
}

fn make_task() -> TaskDto {
    TaskDto {
        gid: "task-1".to_string(),
        name: "Ship release".to_string(),
        completed: false,
        due_on: Some("2026-06-10".to_string()),
        start_on: Some("2026-06-01".to_string()),
        assignee: Some(UserDto {
            gid: "user-1".to_string(),
            name: Some("Alex".to_string()),
            display_name: Some("Alex".to_string()),
        }),
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "project-1".to_string(),
                name: "Inbox".to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }),
        }],
        custom_fields: vec![CustomFieldValueDto {
            gid: "custom-1".to_string(),
            name: "Priority".to_string(),
            display_value: Some("High".to_string()),
            enum_value: None,
        }],
    }
}

#[test]
fn pressing_m_displays_the_task_view() {
    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
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
        .with_tasks("project-1", vec![make_task()]);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    };
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_project_list_session(&mut app, &mut source, &mut terminal).expect("session runs");

    assert!(app.tasks.visible());
    assert_eq!(app.tasks.focus_mode(), tuisana::app::task_review::TaskFocusMode::Tasks);
    assert_eq!(app.tasks.table().rows.len(), 1);

    let buffer = terminal.backend_mut().buffer().clone();
    let text = buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("Task review"));
    assert!(text.contains("Tasks"));
    assert!(text.contains("Ship rele"));
    assert!(text.contains("Today"));
    assert!(text.contains("Priority"));
}
