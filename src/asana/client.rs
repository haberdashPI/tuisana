//! HTTP-backed Asana client implementation.
//!
//! This module turns the app's abstract `AsanaClient` trait into concrete
//! requests against the public Asana REST API.

use reqwest::blocking::Client;
use serde_json::Value;

use crate::{
    asana::{
        dto::{
            CollectionResponse, ProjectCustomFieldSettingDto, ProjectDto, ResourceResponse,
            SectionDto, TaskDto, UserDto,
        },
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget,
    },
    config::AuthConfig,
    domain::{Project, TaskFieldEdit},
    error::{Error, Result},
};

const ASANA_API_BASE_URL: &str = "https://app.asana.com/api/1.0";

/// The task fields every task request asks Asana for.
///
/// Shared by the list, subtask, and single-task endpoints so a record built
/// from any of them carries the same information — in particular `parent.gid`,
/// which is what lets a subtask fetched by assignee be traced back to the
/// project its parent lives in.
const TASK_OPT_FIELDS: &str = "gid,name,completed,modified_at,due_on,start_on,assignee.gid,assignee.name,num_subtasks,parent.gid,memberships.project.gid,memberships.project.name,memberships.section.gid,memberships.section.name,custom_fields.gid,custom_fields.name,custom_fields.display_value,custom_fields.enum_value.gid,custom_fields.enum_value.name";

/// Minimal transport abstraction for JSON requests.
pub trait Transport {
    fn get_json(&self, path: &str, query: &[(&str, String)], token: &str) -> Result<Value>;
    /// Sends a JSON body and returns the response.
    fn put_json(&self, path: &str, body: &Value, token: &str) -> Result<Value>;
}

/// Production HTTP transport using `reqwest`.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: Client,
    base_url: String,
}

impl ReqwestTransport {
    /// Builds a transport configured for the public Asana API.
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            base_url: ASANA_API_BASE_URL.to_string(),
        })
    }
}

impl Transport for ReqwestTransport {
    fn get_json(&self, path: &str, query: &[(&str, String)], token: &str) -> Result<Value> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "application/json")
            .query(query)
            .send()
            .map_err(|err| Error::Backend(format!("asana request failed: {err}")))?;

        let response = response
            .error_for_status()
            .map_err(|err| Error::Backend(format!("asana request failed: {err}")))?;

        response
            .json::<Value>()
            .map_err(|err| Error::Backend(format!("failed to decode asana response: {err}")))
    }

    fn put_json(&self, path: &str, body: &Value, token: &str) -> Result<Value> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let response = self
            .client
            .put(url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "application/json")
            .json(body)
            .send()
            .map_err(|err| Error::Backend(format!("asana request failed: {err}")))?;

        let response = response
            .error_for_status()
            .map_err(|err| Error::Backend(format!("asana request failed: {err}")))?;

        response
            .json::<Value>()
            .map_err(|err| Error::Backend(format!("failed to decode asana response: {err}")))
    }
}

/// The `data` object one field change sends.
///
/// A cleared value is JSON `null`, which is how Asana unsets a field: omitting
/// the key means "no change", which would make clearing impossible.
fn update_body(edit: &TaskFieldEdit) -> Value {
    use crate::domain::CustomFieldValue;
    use serde_json::json;

    match edit {
        TaskFieldEdit::Name(name) => json!({ "name": name }),
        TaskFieldEdit::Completed(completed) => json!({ "completed": completed }),
        TaskFieldEdit::Due(date) => json!({ "due_on": date }),
        TaskFieldEdit::Start(date) => json!({ "start_on": date }),
        TaskFieldEdit::Assignee(assignee) => {
            json!({ "assignee": assignee.as_ref().map(|assignee| assignee.handle.clone()) })
        }
        TaskFieldEdit::CustomField { gid, value } => {
            let value = match value {
                Some(CustomFieldValue::Enum { option_gid, .. }) => Value::from(option_gid.clone()),
                Some(CustomFieldValue::Text(text)) => Value::from(text.clone()),
                Some(CustomFieldValue::Number { value, .. }) => Value::from(*value),
                None => Value::Null,
            };
            json!({ "custom_fields": { gid.clone(): value } })
        }
    }
}

/// HTTP implementation of the app's `AsanaClient` trait.
#[derive(Clone)]
pub struct HttpAsanaClient<T = ReqwestTransport> {
    transport: T,
    personal_access_token: String,
    workspace_gid: Option<String>,
}

impl HttpAsanaClient<ReqwestTransport> {
    /// Builds an HTTP client from the loaded auth configuration.
    pub fn from_config(config: &AuthConfig) -> Result<Self> {
        Ok(Self {
            transport: ReqwestTransport::new()?,
            personal_access_token: config.personal_access_token.clone(),
            workspace_gid: config.workspace_gid.clone(),
        })
    }
}

impl<T: Transport> HttpAsanaClient<T> {
    #[cfg(test)]
    /// Builds a client with a custom transport for tests.
    pub(crate) fn with_transport(
        transport: T,
        personal_access_token: impl Into<String>,
        workspace_gid: Option<String>,
    ) -> Self {
        Self {
            transport,
            personal_access_token: personal_access_token.into(),
            workspace_gid,
        }
    }

    fn list_projects_page(&self, offset: Option<&str>) -> Result<CollectionResponse<ProjectDto>> {
        let mut query = vec![("archived", "false".to_string()), ("limit", "100".to_string())];
        if let Some(workspace_gid) = &self.workspace_gid {
            query.push(("workspace", workspace_gid.clone()));
        }
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self
            .transport
            .get_json("projects", &query, &self.personal_access_token)?;

        serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode project list: {err}")))
    }

    fn list_tasks_page(
        &self,
        task_query: &TaskQuery,
        offset: Option<&str>,
    ) -> Result<CollectionResponse<TaskDto>> {
        let mut query = vec![
            ("opt_fields", TASK_OPT_FIELDS.to_string()),
            ("limit", "100".to_string()),
        ];
        if matches!(task_query.scope, TaskLoadScope::OpenOnly) {
            query.push(("completed_since", "now".to_string()));
        }
        if let Some(date) = &task_query.due_after {
            query.push(("due_on.after", date.clone()));
        }
        if let Some(date) = &task_query.due_before {
            query.push(("due_on.before", date.clone()));
        }

        let path = match &task_query.target {
            TaskTarget::Project(project_gid) => format!("projects/{project_gid}/tasks"),
            TaskTarget::AssignedToMe(user_gid) => {
                let workspace = self.workspace_gid.as_deref().ok_or_else(|| {
                    Error::Backend(
                        "auth.workspace_gid must be set to fetch assigned-to-me tasks".to_string(),
                    )
                })?;
                query.push(("assignee", user_gid.clone()));
                query.push(("workspace", workspace.to_string()));
                "tasks".to_string()
            }
        };
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self
            .transport
            .get_json(&path, &query, &self.personal_access_token)?;

        serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode task list: {err}")))
    }

    fn list_subtasks_page(
        &self,
        task_gid: &str,
        scope: TaskLoadScope,
        offset: Option<&str>,
    ) -> Result<CollectionResponse<TaskDto>> {
        let mut query = vec![
            ("opt_fields", TASK_OPT_FIELDS.to_string()),
            ("limit", "100".to_string()),
        ];
        if matches!(scope, TaskLoadScope::OpenOnly) {
            query.push(("completed_since", "now".to_string()));
        }
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self.transport.get_json(
            &format!("tasks/{task_gid}/subtasks"),
            &query,
            &self.personal_access_token,
        )?;

        serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode subtask list: {err}")))
    }

    fn list_sections_page(
        &self,
        project_gid: &str,
        offset: Option<&str>,
    ) -> Result<CollectionResponse<SectionDto>> {
        let mut query = vec![
            ("opt_fields", "gid,name".to_string()),
            ("limit", "100".to_string()),
        ];
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self.transport.get_json(
            &format!("projects/{project_gid}/sections"),
            &query,
            &self.personal_access_token,
        )?;

        serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode section list: {err}")))
    }

    fn list_custom_field_settings_page(
        &self,
        project_gid: &str,
        offset: Option<&str>,
    ) -> Result<CollectionResponse<ProjectCustomFieldSettingDto>> {
        let mut query = vec![
            (
                "opt_fields",
                "gid,custom_field.gid,custom_field.name,\
                 custom_field.resource_subtype,custom_field.enum_options.gid,\
                 custom_field.enum_options.name,custom_field.enum_options.enabled"
                    .replace(' ', ""),
            ),
            ("limit", "100".to_string()),
        ];
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self.transport.get_json(
            &format!("projects/{project_gid}/custom_field_settings"),
            &query,
            &self.personal_access_token,
        )?;

        serde_json::from_value(json).map_err(|err| {
            Error::Backend(format!("failed to decode custom field settings: {err}"))
        })
    }
}

impl<T: Transport> AsanaClient for HttpAsanaClient<T> {
    fn list_projects(&self) -> Result<Vec<Project>> {
        let mut projects = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_projects_page(offset.as_deref())?;
            projects.extend(
                page.data
                    .into_iter()
                    .map(|project| Project::new(project.gid, project.name, false)),
            );

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(projects)
    }

    fn list_tasks(&self, query: &TaskQuery) -> Result<Vec<TaskDto>> {
        let mut tasks = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_tasks_page(query, offset.as_deref())?;
            tasks.extend(page.data);

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(tasks)
    }

    fn list_subtasks(&self, task_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>> {
        let mut tasks = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_subtasks_page(task_gid, scope, offset.as_deref())?;
            tasks.extend(page.data);

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(tasks)
    }

    fn get_task(&self, task_gid: &str) -> Result<TaskDto> {
        let json = self.transport.get_json(
            &format!("tasks/{task_gid}"),
            &[("opt_fields", TASK_OPT_FIELDS.to_string())],
            &self.personal_access_token,
        )?;

        let response: ResourceResponse<TaskDto> = serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode task {task_gid}: {err}")))?;
        Ok(response.data)
    }

    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>> {
        let mut sections = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_sections_page(project_gid, offset.as_deref())?;
            sections.extend(page.data);

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(sections)
    }

    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>> {
        let mut settings = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_custom_field_settings_page(project_gid, offset.as_deref())?;
            settings.extend(page.data);

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(settings)
    }

    fn update_task(&self, task_gid: &str, edit: &TaskFieldEdit) -> Result<TaskDto> {
        let body = serde_json::json!({ "data": update_body(edit) });
        let json = self.transport.put_json(
            &format!("tasks/{task_gid}?opt_fields={TASK_OPT_FIELDS}"),
            &body,
            &self.personal_access_token,
        )?;

        let response: ResourceResponse<TaskDto> = serde_json::from_value(json).map_err(|err| {
            Error::Backend(format!("failed to decode updated task {task_gid}: {err}"))
        })?;
        Ok(response.data)
    }

    fn current_user_gid(&self) -> Result<String> {
        let json = self.transport.get_json(
            "users/me",
            &[("opt_fields", "gid".to_string())],
            &self.personal_access_token,
        )?;

        let response: ResourceResponse<UserDto> = serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode current user: {err}")))?;
        Ok(response.data.gid)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::{HttpAsanaClient, Transport, TASK_OPT_FIELDS};
    use crate::asana::{AsanaClient, TaskLoadScope, TaskQuery};
    use crate::domain::TaskFieldEdit;
    use crate::error::{Error, Result};

    struct MockTransport {
        requests: RefCell<Vec<(String, Vec<(String, String)>, String)>>,
        /// Every `put_json` call, as `(path, body)`.
        puts: RefCell<Vec<(String, serde_json::Value)>>,
        responses: RefCell<Vec<serde_json::Value>>,
    }

    impl MockTransport {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                puts: RefCell::new(Vec::new()),
                responses: RefCell::new(responses),
            }
        }
    }

    impl Transport for MockTransport {
        fn get_json(
            &self,
            path: &str,
            query: &[(&str, String)],
            token: &str,
        ) -> Result<serde_json::Value> {
            self.requests.borrow_mut().push((
                path.to_string(),
                query
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), value.clone()))
                    .collect(),
                token.to_string(),
            ));
            Ok(self.responses.borrow_mut().remove(0))
        }

        fn put_json(
            &self,
            path: &str,
            body: &serde_json::Value,
            _token: &str,
        ) -> Result<serde_json::Value> {
            self.puts
                .borrow_mut()
                .push((path.to_string(), body.clone()));
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    #[test]
    fn lists_projects_with_auth_and_workspace_filters() {
        let transport = MockTransport::new(vec![json!({
            "data": [
                { "gid": "1", "name": "Inbox" }
            ],
            "next_page": null
        })]);
        let client =
            HttpAsanaClient::with_transport(transport, "pat_123", Some("ws_42".to_string()));

        let projects = client.list_projects().expect("projects load");

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Inbox");
    }

    #[test]
    fn assigned_to_me_query_hits_the_tasks_endpoint_with_assignee_and_workspace() {
        let transport = MockTransport::new(vec![json!({ "data": [], "next_page": null })]);
        let client =
            HttpAsanaClient::with_transport(transport, "pat_123", Some("ws_42".to_string()));

        let tasks = client
            .list_tasks(&TaskQuery::for_assigned_to_me("user_1", TaskLoadScope::All))
            .expect("tasks load");
        assert!(tasks.is_empty());

        let requests = client.transport.requests.borrow();
        let (path, query, _) = &requests[0];
        assert_eq!(path, "tasks");
        assert!(query.contains(&("assignee".to_string(), "user_1".to_string())));
        assert!(query.contains(&("workspace".to_string(), "ws_42".to_string())));
    }

    #[test]
    fn assigned_to_me_query_fails_without_workspace_gid() {
        let transport = MockTransport::new(vec![]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let err = client
            .list_tasks(&TaskQuery::for_assigned_to_me("user_1", TaskLoadScope::All))
            .expect_err("missing workspace_gid should fail");

        assert!(matches!(err, Error::Backend(message) if message.contains("workspace_gid")));
    }

    #[test]
    fn get_task_fetches_one_task_with_its_parent_and_memberships() {
        let transport = MockTransport::new(vec![json!({
            "data": {
                "gid": "t1",
                "name": "Parent in Zeta",
                "parent": { "gid": "t0" },
                "memberships": [
                    { "project": { "gid": "pz", "name": "Zeta" }, "section": null }
                ]
            }
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let task = client.get_task("t1").expect("task resolves");

        assert_eq!(task.name, "Parent in Zeta");
        assert_eq!(task.parent.expect("a parent").gid, "t0");
        assert_eq!(task.memberships[0].project.name, "Zeta");

        let requests = client.transport.requests.borrow();
        let (path, query, _) = &requests[0];
        assert_eq!(path, "tasks/t1");
        assert!(query.contains(&("opt_fields".to_string(), TASK_OPT_FIELDS.to_string())));
    }

    /// A subtask can only be traced back to its parent's project when the
    /// parent id comes back with the task, so every task request must ask for it.
    #[test]
    fn task_requests_ask_for_the_parent_id() {
        assert!(TASK_OPT_FIELDS.contains("parent.gid"));
    }

    #[test]
    fn an_update_sends_one_field_and_asks_for_the_task_back() {
        let transport = MockTransport::new(vec![json!({
            "data": { "gid": "t1", "name": "Renamed", "modified_at": "2026-06-02T00:00:00Z" }
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let task = client
            .update_task("t1", &TaskFieldEdit::Name("Renamed".to_string()))
            .expect("the task updates");

        assert_eq!(task.name, "Renamed");
        let puts = client.transport.puts.borrow();
        let (path, body) = &puts[0];
        assert!(path.starts_with("tasks/t1?"), "{path}");
        assert!(
            path.contains(TASK_OPT_FIELDS),
            "the reply has to decode as an ordinary task"
        );
        assert_eq!(body, &json!({ "data": { "name": "Renamed" } }));
    }

    #[test]
    fn a_cleared_value_is_sent_as_null_rather_than_omitted() {
        // Omitting the key means "no change", which would make clearing a
        // field impossible.
        let responses = vec![
            json!({ "data": { "gid": "t1", "name": "Ship" } }),
            json!({ "data": { "gid": "t1", "name": "Ship" } }),
            json!({ "data": { "gid": "t1", "name": "Ship" } }),
        ];
        let transport = MockTransport::new(responses);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        client
            .update_task("t1", &TaskFieldEdit::Due(None))
            .expect("the date clears");
        client
            .update_task("t1", &TaskFieldEdit::Assignee(None))
            .expect("the assignee clears");
        client
            .update_task(
                "t1",
                &TaskFieldEdit::CustomField {
                    gid: "cf1".to_string(),
                    value: None,
                },
            )
            .expect("the custom field clears");

        let puts = client.transport.puts.borrow();
        assert_eq!(puts[0].1, json!({ "data": { "due_on": null } }));
        assert_eq!(puts[1].1, json!({ "data": { "assignee": null } }));
        assert_eq!(
            puts[2].1,
            json!({ "data": { "custom_fields": { "cf1": null } } })
        );
    }

    #[test]
    fn an_enum_custom_field_is_sent_as_its_option_gid() {
        let transport = MockTransport::new(vec![json!({
            "data": { "gid": "t1", "name": "Ship" }
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        client
            .update_task(
                "t1",
                &TaskFieldEdit::CustomField {
                    gid: "cf1".to_string(),
                    value: Some(crate::domain::CustomFieldValue::Enum {
                        option_gid: "opt-high".to_string(),
                        name: "High".to_string(),
                    }),
                },
            )
            .expect("the field updates");

        assert_eq!(
            client.transport.puts.borrow()[0].1,
            json!({ "data": { "custom_fields": { "cf1": "opt-high" } } }),
            "the name is for the table; the gid is what Asana takes"
        );
    }

    /// A picker can only offer an option the settings request asked for.
    #[test]
    fn the_settings_request_asks_for_declared_enum_options() {
        let transport = MockTransport::new(vec![json!({ "data": [], "next_page": null })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        client
            .list_project_custom_field_settings("p1")
            .expect("settings load");

        let requests = client.transport.requests.borrow();
        let opt_fields = requests[0]
            .1
            .iter()
            .find(|(key, _)| key == "opt_fields")
            .map(|(_, value)| value.clone())
            .expect("an opt_fields parameter");
        for field in [
            "custom_field.resource_subtype",
            "custom_field.enum_options.gid",
            "custom_field.enum_options.name",
            "custom_field.enum_options.enabled",
        ] {
            assert!(opt_fields.contains(field), "{opt_fields} is missing {field}");
        }
    }

    #[test]
    fn current_user_gid_resolves_the_logged_in_user_via_users_me() {
        let transport = MockTransport::new(vec![json!({
            "data": { "gid": "user_1", "name": "Alex" }
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let gid = client.current_user_gid().expect("current user resolves");

        assert_eq!(gid, "user_1");
        let requests = client.transport.requests.borrow();
        assert_eq!(requests[0].0, "users/me");
    }
}
