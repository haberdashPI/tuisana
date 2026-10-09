//! HTTP-backed Asana client implementation.
//!
//! This module turns the app's abstract `AsanaClient` trait into concrete
//! requests against the public Asana REST API.

use std::{sync::Arc, time::Duration};

use reqwest::blocking::Client;
use serde_json::Value;

use crate::{
    asana::{
        dto::{
            CollectionResponse, ProjectCustomFieldSettingDto, ProjectDto, ResourceResponse,
            SectionDto, TaskDto, UserDto,
        },
        throttle::WriteThrottle,
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget, TaskWrite, MAX_BATCH_ACTIONS,
    },
    config::AuthConfig,
    domain::{NewTask, Project, ProjectEdit, ProjectMembership, TaskFieldEdit},
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
    /// The same, for the endpoints that are verbs rather than fields.
    ///
    /// Project membership is `tasks/{gid}/addProject`, not a key in a task
    /// patch, so it cannot go through [`Transport::put_json`].
    fn post_json(&self, path: &str, body: &Value, token: &str) -> Result<Value>;
    /// Deletes a resource. Beside the three above for the same reason they
    /// are beside each other: a verb the other methods cannot spell.
    fn delete_json(&self, path: &str, token: &str) -> Result<Value>;
}

/// Production HTTP transport using `reqwest`.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: Client,
    base_url: String,
}

/// How many times a rate-limited request is re-sent before giving up.
///
/// Asana's `Retry-After` is usually a second or two, so three attempts covers
/// the ordinary case. Past that the limiter is telling us something the client
/// cannot fix by waiting longer, and the error belongs on screen.
const RATE_LIMIT_ATTEMPTS: usize = 3;
/// Longest a single `Retry-After` will be honoured.
///
/// A header asking for five minutes would freeze the worker thread holding it
/// — and the write budget with it — for five minutes. Past this the request
/// fails and says so.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);
/// What a `429` with no usable `Retry-After` waits.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(2);

impl ReqwestTransport {
    /// Builds a transport configured for the public Asana API.
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            base_url: ASANA_API_BASE_URL.to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Sends a request, waiting out a `429` and trying again.
    ///
    /// `build` is a closure rather than a `RequestBuilder` because a retry
    /// needs a *second* request: a builder is consumed by `send`, and
    /// `try_clone` fails on exactly the bodies that would be worth retrying.
    ///
    /// Honouring `Retry-After` is not politeness, it is arithmetic — Asana
    /// counts a rejected request against the quota too, so retrying early
    /// digs the hole deeper. The client-side throttle is what should keep us
    /// from getting here at all; this is the backstop for the case where
    /// something else on the account is spending the same quota.
    fn send_with_retry(
        &self,
        path: &str,
        build: impl Fn() -> reqwest::blocking::RequestBuilder,
    ) -> Result<Value> {
        for attempt in 1..=RATE_LIMIT_ATTEMPTS {
            let response = build()
                .send()
                .map_err(|err| Error::Backend(format!("asana request failed: {err}")))?;

            if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS
                || attempt == RATE_LIMIT_ATTEMPTS
            {
                return decode(response, path);
            }

            let wait = retry_after(response.headers());
            crate::app::debug_log(&format!(
                "asana rate limited on {path}, waiting {:?} (attempt {attempt})",
                wait
            ));
            std::thread::sleep(wait);
        }

        // Unreachable: the loop returns on its last attempt.
        Err(Error::Backend(format!(
            "asana request failed: rate limited for {path}"
        )))
    }
}

/// How long a `429` asked us to wait, clamped to something a UI can survive.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_RETRY_AFTER)
        .min(MAX_RETRY_AFTER)
}

impl Transport for ReqwestTransport {
    fn get_json(&self, path: &str, query: &[(&str, String)], token: &str) -> Result<Value> {
        let url = self.url(path);
        self.send_with_retry(path, || {
            self.client
                .get(&url)
                .bearer_auth(token)
                .header(reqwest::header::ACCEPT, "application/json")
                .query(query)
        })
    }

    fn put_json(&self, path: &str, body: &Value, token: &str) -> Result<Value> {
        let url = self.url(path);
        self.send_with_retry(path, || {
            self.client
                .put(&url)
                .bearer_auth(token)
                .header(reqwest::header::ACCEPT, "application/json")
                .json(body)
        })
    }

    fn post_json(&self, path: &str, body: &Value, token: &str) -> Result<Value> {
        let url = self.url(path);
        self.send_with_retry(path, || {
            self.client
                .post(&url)
                .bearer_auth(token)
                .header(reqwest::header::ACCEPT, "application/json")
                .json(body)
        })
    }

    fn delete_json(&self, path: &str, token: &str) -> Result<Value> {
        let url = self.url(path);
        self.send_with_retry(path, || {
            self.client
                .delete(&url)
                .bearer_auth(token)
                .header(reqwest::header::ACCEPT, "application/json")
        })
    }
}

/// Turns a response into JSON, or into an error carrying what Asana said.
///
/// `error_for_status` reports only the status and the request URL, and an
/// Asana URL drags the whole `opt_fields` list behind it — a hundred
/// characters of field names where the one sentence explaining the refusal
/// should be. That sentence is in the body, so the body is read before the
/// status is turned into an error.
fn decode(response: reqwest::blocking::Response, path: &str) -> Result<Value> {
    let status = response.status();
    if status.is_success() {
        return response
            .json::<Value>()
            .map_err(|err| Error::Backend(format!("failed to decode asana response: {err}")));
    }

    let detail = response
        .text()
        .ok()
        .and_then(|body| asana_error_message(&body))
        .unwrap_or_default();
    let path = path.split('?').next().unwrap_or(path);
    Err(Error::Backend(format!(
        "asana request failed: {status} for {path}{detail}"
    )))
}

/// Pulls the human-readable part out of an Asana error response.
///
/// Asana answers a refusal with `{"errors": [{"message": ...}]}`; anything
/// else — an HTML gateway page, an empty body — is dropped rather than shown,
/// since the status already says as much as it would.
fn asana_error_message(body: &str) -> Option<String> {
    asana_error_detail(&serde_json::from_str::<Value>(body).ok()?)
}

/// The same, for a body that has already been parsed.
///
/// Which is every action in a batch: those arrive as JSON inside a `200`, so
/// there is no text to re-parse.
fn asana_error_detail(body: &Value) -> Option<String> {
    let messages = body
        .get("errors")?
        .as_array()?
        .iter()
        .filter_map(|error| error.get("message")?.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    (!messages.is_empty()).then(|| format!(": {messages}"))
}

/// Turns one entry of a batch response into that write's result.
///
/// The timestamp is read out of the body rather than the whole task being
/// decoded: a successful write's record is already correct locally — the
/// optimistic update wrote it — and `modified_at` is the one thing only the
/// server knows.
fn batch_result(result: &Value) -> Result<Option<String>> {
    let status = result
        .get("status_code")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    if !(200..300).contains(&status) {
        let detail = result
            .get("body")
            .and_then(asana_error_detail)
            .unwrap_or_default();
        return Err(Error::Backend(format!(
            "asana request failed: {status}{detail}"
        )));
    }

    Ok(result
        .get("body")
        .and_then(|body| body.get("data"))
        .and_then(|data| data.get("modified_at"))
        .and_then(Value::as_str)
        .map(str::to_string))
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
        // The due date `start_on` has to be sent with is added by the
        // caller, which is the only place that knows it.
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
    /// The write budget, shared by every clone of this client.
    ///
    /// Behind an `Arc` because the write pool clones the client once per
    /// worker thread: a budget that was cloned with it would be no budget at
    /// all. See [`crate::asana::throttle`] for what it counts and why.
    throttle: Arc<WriteThrottle>,
}

impl HttpAsanaClient<ReqwestTransport> {
    /// Builds an HTTP client from the loaded auth configuration.
    pub fn from_config(config: &AuthConfig) -> Result<Self> {
        Ok(Self {
            transport: ReqwestTransport::new()?,
            personal_access_token: config.personal_access_token.clone(),
            workspace_gid: config.workspace_gid.clone(),
            throttle: Arc::new(WriteThrottle::default()),
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
            // Effectively off: a test that waited out a real rate limit would
            // be a test of `std::thread::sleep`.
            throttle: Arc::new(WriteThrottle::new(usize::MAX, f64::MAX)),
        }
    }

    /// Reads back the due date a start-date write has to restate.
    ///
    /// Asana refuses any request that sets or clears `start_on` without also
    /// naming `due_on` or `due_at`, so a start-date edit has to carry the due
    /// date the task already holds. It is read from the server rather than
    /// from the loaded record because a stale local copy would not merely
    /// fail — it would quietly move the due date. `due_at` wins when the task
    /// has one: sending `due_on` for a task due at a time of day would drop
    /// the time.
    fn current_due(&self, task_gid: &str) -> Result<(&'static str, Value)> {
        let json = self.transport.get_json(
            &format!("tasks/{task_gid}"),
            &[("opt_fields", "due_on,due_at".to_string())],
            &self.personal_access_token,
        )?;
        let data = &json["data"];

        match data.get("due_at") {
            Some(due_at) if !due_at.is_null() => Ok(("due_at", due_at.clone())),
            _ => Ok(("due_on", data.get("due_on").cloned().unwrap_or(Value::Null))),
        }
    }

    /// Turns one write into an entry for the `actions` array.
    ///
    /// `Err` here is a *per-write* failure: the only thing that can go wrong
    /// is the read a start-date edit needs, and one task whose due date could
    /// not be read must not take the other nine down with it.
    fn batch_action(&self, write: &TaskWrite) -> Result<Value> {
        use serde_json::json;

        Ok(match write {
            TaskWrite::Field { gid, edit } => {
                let mut data = update_body(edit);
                if matches!(edit, TaskFieldEdit::Start(_)) {
                    // Not nested in the batch: `/batch` refuses to call
                    // itself, and this is a read the write cannot be built
                    // without. Charged for by `chunk_cost`.
                    let (key, due) = self.current_due(gid)?;
                    if let Some(data) = data.as_object_mut() {
                        data.insert(key.to_string(), due);
                    }
                }
                json!({
                    "relative_path": format!("/tasks/{gid}"),
                    "method": "put",
                    "data": data,
                    // A list of names, not the comma-separated string the
                    // query parameter takes.
                    "options": { "fields": TASK_OPT_FIELDS.split(',').collect::<Vec<_>>() },
                })
            }
            TaskWrite::Project(edit) => {
                let verb = match edit.membership {
                    ProjectMembership::Add => "addProject",
                    ProjectMembership::Remove => "removeProject",
                };
                json!({
                    "relative_path": format!("/tasks/{}/{verb}", edit.gid),
                    "method": "post",
                    "data": { "project": edit.project_gid },
                })
            }
            TaskWrite::Parent { gid, parent_gid } => json!({
                "relative_path": format!("/tasks/{gid}/setParent"),
                "method": "post",
                "data": { "parent": parent_gid },
            }),
            TaskWrite::Delete { gid } => json!({
                "relative_path": format!("/tasks/{gid}"),
                "method": "delete",
            }),
        })
    }

    /// Sends one chunk of actions and answers one result per action, in order.
    ///
    /// Asana answers a batch with `200` and an array of per-action results
    /// even when every action in it failed, so the statuses in the body are
    /// the only place a refusal is reported.
    fn send_batch(&self, actions: Vec<Value>) -> Result<Vec<Result<Option<String>>>> {
        let count = actions.len();
        let body = serde_json::json!({ "data": { "actions": actions } });
        let json = self
            .transport
            .post_json("batch", &body, &self.personal_access_token)?;

        let results = json
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Backend("failed to decode batch response".to_string()))?;

        Ok((0..count)
            .map(|index| match results.get(index) {
                Some(result) => batch_result(result),
                // Asana returns results positionally, so a short array is not
                // something to guess about.
                None => Err(Error::Backend(format!(
                    "asana answered {} of {count} batched writes",
                    results.len()
                ))),
            })
            .collect())
    }

    /// What the throttle is charged for a chunk, in actions.
    ///
    /// Every write is one action, plus one for each read a start-date edit
    /// has to make first — those reads spend the same per-minute quota, and a
    /// budget that ignored them would be wrong by a factor of two on exactly
    /// the edit most likely to be done in bulk.
    fn chunk_cost(writes: &[TaskWrite]) -> usize {
        writes.len() + writes.iter().filter(|write| write.needs_read()).count()
    }

    fn list_projects_page(&self, offset: Option<&str>) -> Result<CollectionResponse<ProjectDto>> {
        let mut query = vec![
            // The workspace travels with the project so that creating a task
            // outside every project still has one to name. `auth.workspace_gid`
            // is optional, so it cannot be relied on.
            ("opt_fields", "gid,name,workspace.gid".to_string()),
            ("archived", "false".to_string()),
            ("limit", "100".to_string()),
        ];
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

    fn list_users_page(&self, offset: Option<&str>) -> Result<CollectionResponse<UserDto>> {
        let mut query = vec![
            ("opt_fields", "gid,name".to_string()),
            ("limit", "100".to_string()),
        ];
        // The endpoint is workspace-scoped: without one it answers with every
        // user the token can see across every workspace, which is a different
        // question from "who can I assign this to".
        let workspace = self.workspace_gid.as_deref().ok_or_else(|| {
            Error::Backend("auth.workspace_gid must be set to list users".to_string())
        })?;
        query.push(("workspace", workspace.to_string()));
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self
            .transport
            .get_json("users", &query, &self.personal_access_token)?;

        serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode user list: {err}")))
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
            projects.extend(page.data.into_iter().map(|project| {
                let workspace = project.workspace.map(|workspace| workspace.gid);
                Project::new(project.gid, project.name, false).in_workspace(workspace)
            }));

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
        let mut data = update_body(edit);
        if matches!(edit, TaskFieldEdit::Start(_)) {
            let (key, due) = self.current_due(task_gid)?;
            if let Some(data) = data.as_object_mut() {
                data.insert(key.to_string(), due);
            }
        }

        let body = serde_json::json!({ "data": data });
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

    fn list_users(&self) -> Result<Vec<UserDto>> {
        let mut users = Vec::new();
        let mut offset: Option<String> = None;

        loop {
            let page = self.list_users_page(offset.as_deref())?;
            users.extend(page.data);

            match page.next_page {
                Some(next_page) => offset = Some(next_page.offset),
                None => break,
            }
        }

        Ok(users)
    }

    fn update_task_project(&self, edit: &ProjectEdit) -> Result<()> {
        let verb = match edit.membership {
            ProjectMembership::Add => "addProject",
            ProjectMembership::Remove => "removeProject",
        };
        let body = serde_json::json!({ "data": { "project": edit.project_gid } });
        self.transport.post_json(
            &format!("tasks/{}/{verb}", edit.gid),
            &body,
            &self.personal_access_token,
        )?;
        Ok(())
    }

    fn create_task(&self, task: &NewTask) -> Result<TaskDto> {
        let mut data = serde_json::Map::new();
        data.insert("name".to_string(), Value::from(task.name.clone()));
        if let Some(parent) = &task.parent_gid {
            data.insert("parent".to_string(), Value::from(parent.clone()));
        }
        if let Some(project) = &task.project_gid {
            data.insert(
                "projects".to_string(),
                Value::Array(vec![Value::from(project.clone())]),
            );
        }
        if let Some(assignee) = &task.assignee_gid {
            data.insert("assignee".to_string(), Value::from(assignee.clone()));
        }
        // Asana wants exactly one of these. A task with a parent or a project
        // already has its workspace decided, and naming it again is refused.
        if task.parent_gid.is_none() && task.project_gid.is_none() {
            let workspace = task.workspace_gid.as_deref().ok_or_else(|| {
                Error::Backend(
                    "a task in no project needs a workspace, and none is known".to_string(),
                )
            })?;
            data.insert("workspace".to_string(), Value::from(workspace.to_string()));
        }

        let body = serde_json::json!({ "data": Value::Object(data) });
        let json = self.transport.post_json(
            &format!("tasks?opt_fields={TASK_OPT_FIELDS}"),
            &body,
            &self.personal_access_token,
        )?;
        let response: ResourceResponse<TaskDto> = serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode created task: {err}")))?;

        // Second, and only on the first's success: a section is not a field
        // on create.
        if let Some(section) = &task.section_gid {
            self.add_task_to_section(section, &response.data.gid)?;
        }
        Ok(response.data)
    }

    fn delete_task(&self, task_gid: &str) -> Result<()> {
        self.transport
            .delete_json(&format!("tasks/{task_gid}"), &self.personal_access_token)?;
        Ok(())
    }

    fn set_task_parent(&self, task_gid: &str, parent_gid: Option<&str>) -> Result<()> {
        let body = serde_json::json!({ "data": { "parent": parent_gid } });
        self.transport.post_json(
            &format!("tasks/{task_gid}/setParent"),
            &body,
            &self.personal_access_token,
        )?;
        Ok(())
    }

    /// Sends a chunk of writes as one `/batch` request, under the throttle.
    ///
    /// The batch buys round trips, not quota: Asana counts a ten-action batch
    /// as ten requests against both the per-minute and the concurrent
    /// limiters. Which is why the throttle is acquired here rather than left
    /// to the caller — the one place that knows how many actions are about to
    /// be spent is the one building them.
    ///
    /// A chunk longer than [`MAX_BATCH_ACTIONS`] would be refused outright,
    /// so it is split rather than sent; callers that already chunk pay
    /// nothing for the check.
    fn write_tasks(&self, writes: &[TaskWrite]) -> Vec<Result<Option<String>>> {
        let mut results = Vec::with_capacity(writes.len());

        for chunk in writes.chunks(MAX_BATCH_ACTIONS) {
            // Taken before the actions are built, because building them is
            // where a start-date edit's read is made — and that read is part
            // of what the chunk costs.
            let _budget = self.throttle.acquire(Self::chunk_cost(chunk));

            // Built first, so a write that cannot be expressed at all fails
            // on its own rather than as part of the batch. `sent` maps a
            // position in the request back to a position in `chunk`.
            let mut actions = Vec::with_capacity(chunk.len());
            let mut sent = Vec::with_capacity(chunk.len());
            let mut chunk_results: Vec<Option<Result<Option<String>>>> =
                chunk.iter().map(|_| None).collect();
            for (index, write) in chunk.iter().enumerate() {
                match self.batch_action(write) {
                    Ok(action) => {
                        actions.push(action);
                        sent.push(index);
                    }
                    Err(err) => chunk_results[index] = Some(Err(err)),
                }
            }

            if !actions.is_empty() {
                match self.send_batch(actions) {
                    Ok(replies) => {
                        for (index, reply) in sent.iter().zip(replies) {
                            chunk_results[*index] = Some(reply);
                        }
                    }
                    // The request itself failed, so every write in it did.
                    // Cloned as text because `Error` is not `Clone` — and the
                    // message is the whole of what the notice shows.
                    Err(err) => {
                        let message = err.to_string();
                        for index in &sent {
                            chunk_results[*index] =
                                Some(Err(Error::Backend(message.clone())));
                        }
                    }
                }
            }

            results.extend(chunk_results.into_iter().map(|result| {
                result.unwrap_or_else(|| {
                    Err(Error::Backend("write was never sent".to_string()))
                })
            }));
        }

        results
    }

    fn add_task_to_section(&self, section_gid: &str, task_gid: &str) -> Result<()> {
        let body = serde_json::json!({ "data": { "task": task_gid } });
        self.transport.post_json(
            &format!("sections/{section_gid}/addTask"),
            &body,
            &self.personal_access_token,
        )?;
        Ok(())
    }

    fn create_section(
        &self,
        project_gid: &str,
        name: &str,
        insert_after: Option<&str>,
    ) -> Result<SectionDto> {
        let mut data = serde_json::Map::new();
        data.insert("name".to_string(), Value::from(name.to_string()));
        if let Some(after) = insert_after {
            data.insert("insert_after".to_string(), Value::from(after.to_string()));
        }

        let body = serde_json::json!({ "data": Value::Object(data) });
        let json = self.transport.post_json(
            &format!("projects/{project_gid}/sections"),
            &body,
            &self.personal_access_token,
        )?;
        let response: ResourceResponse<SectionDto> = serde_json::from_value(json)
            .map_err(|err| Error::Backend(format!("failed to decode created section: {err}")))?;
        Ok(response.data)
    }

    fn delete_section(&self, section_gid: &str) -> Result<()> {
        self.transport.delete_json(
            &format!("sections/{section_gid}"),
            &self.personal_access_token,
        )?;
        Ok(())
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

    use super::{asana_error_message, retry_after, HttpAsanaClient, Transport, TASK_OPT_FIELDS};
    use crate::asana::{AsanaClient, TaskLoadScope, TaskQuery, TaskWrite};
    use crate::domain::{ProjectEdit, TaskFieldEdit};
    use crate::error::{Error, Result};

    struct MockTransport {
        requests: RefCell<Vec<(String, Vec<(String, String)>, String)>>,
        /// Every `put_json` call, as `(path, body)`.
        puts: RefCell<Vec<(String, serde_json::Value)>>,
        /// Every `post_json` call, as `(path, body)`.
        posts: RefCell<Vec<(String, serde_json::Value)>>,
        /// Every `delete_json` call, as a path.
        deletes: RefCell<Vec<String>>,
        responses: RefCell<Vec<serde_json::Value>>,
    }

    impl MockTransport {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
                puts: RefCell::new(Vec::new()),
                posts: RefCell::new(Vec::new()),
                deletes: RefCell::new(Vec::new()),
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

        fn post_json(
            &self,
            path: &str,
            body: &serde_json::Value,
            _token: &str,
        ) -> Result<serde_json::Value> {
            self.posts
                .borrow_mut()
                .push((path.to_string(), body.clone()));
            Ok(self.responses.borrow_mut().remove(0))
        }

        fn delete_json(&self, path: &str, _token: &str) -> Result<serde_json::Value> {
            // A delete has no body, so it is logged as one with an empty
            // object: the tests ask which path went out, not what was in it.
            self.deletes.borrow_mut().push(path.to_string());
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    /// One entry of a `/batch` reply, as Asana shapes it.
    fn batch_reply(status: u64, body: serde_json::Value) -> serde_json::Value {
        json!({ "status_code": status, "headers": {}, "body": body })
    }

    #[test]
    fn a_run_of_writes_goes_out_as_one_batch_naming_each_endpoint() {
        let transport = MockTransport::new(vec![json!({
            "data": [
                batch_reply(200, json!({ "data": { "gid": "t1", "modified_at": "2026-10-01T09:00:00Z" } })),
                batch_reply(200, json!({ "data": {} })),
                batch_reply(200, json!({ "data": {} })),
                batch_reply(200, json!({ "data": {} })),
            ]
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let results = client.write_tasks(&[
            TaskWrite::Field {
                gid: "t1".to_string(),
                edit: TaskFieldEdit::Completed(true),
            },
            TaskWrite::Project(ProjectEdit::remove("t2", "p9", "Backlog")),
            TaskWrite::Parent {
                gid: "t3".to_string(),
                parent_gid: None,
            },
            TaskWrite::Delete {
                gid: "t4".to_string(),
            },
        ]);

        // One request for four writes, which is the whole reason the batch
        // endpoint is worth the second code path.
        let posts = client.transport.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, "batch");

        let actions = posts[0].1["data"]["actions"]
            .as_array()
            .expect("an actions array");
        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0]["relative_path"], "/tasks/t1");
        assert_eq!(actions[0]["method"], "put");
        assert_eq!(actions[0]["data"]["completed"], true);
        assert_eq!(actions[1]["relative_path"], "/tasks/t2/removeProject");
        assert_eq!(actions[1]["method"], "post");
        assert_eq!(actions[1]["data"]["project"], "p9");
        assert_eq!(actions[2]["relative_path"], "/tasks/t3/setParent");
        assert_eq!(actions[2]["data"]["parent"], serde_json::Value::Null);
        assert_eq!(actions[3]["relative_path"], "/tasks/t4");
        assert_eq!(actions[3]["method"], "delete");

        // The field write's timestamp comes back; the three verb endpoints
        // have none to give.
        assert_eq!(
            results[0].as_ref().expect("the field write"),
            &Some("2026-10-01T09:00:00Z".to_string())
        );
        assert!(results[1..].iter().all(|result| matches!(result, Ok(None))));
    }

    #[test]
    fn a_batch_longer_than_the_endpoints_cap_is_split() {
        let page = |count: usize| {
            json!({
                "data": (0..count)
                    .map(|_| batch_reply(200, json!({ "data": {} })))
                    .collect::<Vec<_>>()
            })
        };
        let transport = MockTransport::new(vec![page(10), page(2)]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let writes = (0..12)
            .map(|index| TaskWrite::Delete {
                gid: format!("t{index}"),
            })
            .collect::<Vec<_>>();
        let results = client.write_tasks(&writes);

        assert_eq!(results.len(), 12, "one result per write, across both");
        assert!(results.iter().all(Result::is_ok));
        let posts = client.transport.posts.borrow();
        assert_eq!(posts.len(), 2, "ten and then two");
        assert_eq!(posts[0].1["data"]["actions"].as_array().unwrap().len(), 10);
        assert_eq!(posts[1].1["data"]["actions"].as_array().unwrap().len(), 2);
    }

    /// Asana answers a batch with `200` whatever the actions did, so the only
    /// place a refusal is reported is the per-action status — and a batch
    /// where one of three failed has to roll back exactly that one.
    #[test]
    fn one_refused_action_fails_alone() {
        let transport = MockTransport::new(vec![json!({
            "data": [
                batch_reply(200, json!({ "data": {} })),
                batch_reply(403, json!({ "errors": [{ "message": "not your task" }] })),
                batch_reply(200, json!({ "data": {} })),
            ]
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let results = client.write_tasks(&[
            TaskWrite::Delete { gid: "t1".to_string() },
            TaskWrite::Delete { gid: "t2".to_string() },
            TaskWrite::Delete { gid: "t3".to_string() },
        ]);

        assert!(results[0].is_ok());
        assert_eq!(
            results[1].as_ref().expect_err("the refusal").to_string(),
            "backend error: asana request failed: 403: not your task"
        );
        assert!(results[2].is_ok());
    }

    #[test]
    fn a_short_reply_fails_the_writes_it_said_nothing_about() {
        // Rather than guessing: the results are positional, so a missing one
        // cannot be matched to a write by anything but its index.
        let transport = MockTransport::new(vec![json!({
            "data": [batch_reply(200, json!({ "data": {} }))]
        })]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let results = client.write_tasks(&[
            TaskWrite::Delete { gid: "t1".to_string() },
            TaskWrite::Delete { gid: "t2".to_string() },
        ]);

        assert!(results[0].is_ok());
        assert!(results[1]
            .as_ref()
            .expect_err("no reply")
            .to_string()
            .contains("answered 1 of 2"));
    }

    /// Asana refuses a `start_on` that does not restate the due date, and
    /// `/batch` refuses to call itself — so the read has to happen before the
    /// actions are built, one per start-date write.
    #[test]
    fn a_batched_start_date_restates_the_due_date_it_read_first() {
        let transport = MockTransport::new(vec![
            json!({ "data": { "due_on": "2026-10-20", "due_at": null } }),
            json!({ "data": [batch_reply(200, json!({ "data": {} }))] }),
        ]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        let results = client.write_tasks(&[TaskWrite::Field {
            gid: "t1".to_string(),
            edit: TaskFieldEdit::Start(Some("2026-10-15".to_string())),
        }]);

        assert!(results[0].is_ok());
        assert_eq!(client.transport.requests.borrow()[0].0, "tasks/t1");
        let posts = client.transport.posts.borrow();
        let action = &posts[0].1["data"]["actions"][0];
        assert_eq!(action["data"]["start_on"], "2026-10-15");
        assert_eq!(action["data"]["due_on"], "2026-10-20");
    }

    #[test]
    fn a_retry_after_header_is_honoured_and_clamped() {
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));
        assert_eq!(retry_after(&headers).as_secs(), 3);

        // A header asking for five minutes would freeze the worker holding
        // the write budget for five minutes.
        headers.insert(RETRY_AFTER, HeaderValue::from_static("300"));
        assert_eq!(retry_after(&headers).as_secs(), 30);

        // A `429` with no usable header still waits rather than hammering.
        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after(&headers).as_secs(), 2);
        assert_eq!(retry_after(&HeaderMap::new()).as_secs(), 2);
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

    /// Asana refuses `start_on` on its own: the due date has to ride along,
    /// or the write comes back 400 and the start date never moves.
    #[test]
    fn a_start_date_write_restates_the_due_date() {
        let transport = MockTransport::new(vec![
            json!({ "data": { "gid": "t1", "due_on": "2026-09-30", "due_at": null } }),
            json!({ "data": { "gid": "t1", "name": "Ship" } }),
        ]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        client
            .update_task("t1", &TaskFieldEdit::Start(None))
            .expect("the start date clears");

        let (path, query, _) = client.transport.requests.borrow()[0].clone();
        assert_eq!(path, "tasks/t1");
        assert_eq!(
            query,
            vec![("opt_fields".to_string(), "due_on,due_at".to_string())]
        );
        assert_eq!(
            client.transport.puts.borrow()[0].1,
            json!({ "data": { "start_on": null, "due_on": "2026-09-30" } })
        );
    }

    /// Restating a timed due date as `due_on` would drop the time, so the
    /// write names `due_at` whenever the task has one.
    #[test]
    fn a_start_date_write_keeps_a_due_time_intact() {
        let transport = MockTransport::new(vec![
            json!({
                "data": {
                    "gid": "t1",
                    "due_on": "2026-09-30",
                    "due_at": "2026-09-30T17:00:00.000Z"
                }
            }),
            json!({ "data": { "gid": "t1", "name": "Ship" } }),
        ]);
        let client = HttpAsanaClient::with_transport(transport, "pat_123", None);

        client
            .update_task("t1", &TaskFieldEdit::Start(Some("2026-09-01".to_string())))
            .expect("the start date sets");

        assert_eq!(
            client.transport.puts.borrow()[0].1,
            json!({
                "data": { "start_on": "2026-09-01", "due_at": "2026-09-30T17:00:00.000Z" }
            })
        );
    }

    #[test]
    fn a_refusal_is_reported_in_asanas_own_words() {
        let body = json!({
            "errors": [{ "message": "due_on: Must be present when setting start_on" }]
        })
        .to_string();

        assert_eq!(
            asana_error_message(&body).expect("a message"),
            ": due_on: Must be present when setting start_on"
        );
        assert_eq!(asana_error_message("<html>502</html>"), None);
        assert_eq!(
            asana_error_message(&json!({ "errors": [] }).to_string()),
            None
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
    fn a_membership_change_posts_to_the_verb_that_names_it() {
        // Membership is not a field on the task, so it cannot go out as part
        // of a task patch.
        let responses = vec![json!({ "data": {} }), json!({ "data": {} })];
        let client =
            HttpAsanaClient::with_transport(MockTransport::new(responses), "pat_123", None);

        client
            .update_task_project(&crate::domain::ProjectEdit::add("t1", "p2", "Backlog"))
            .expect("the task joins the project");
        client
            .update_task_project(&crate::domain::ProjectEdit::remove("t1", "p1", "Inbox"))
            .expect("and leaves the other");

        let posts = client.transport.posts.borrow();
        assert_eq!(posts[0].0, "tasks/t1/addProject");
        assert_eq!(posts[0].1, json!({ "data": { "project": "p2" } }));
        assert_eq!(posts[1].0, "tasks/t1/removeProject");
        assert_eq!(posts[1].1, json!({ "data": { "project": "p1" } }));
    }

    #[test]
    fn the_user_directory_is_scoped_to_the_workspace() {
        let transport = MockTransport::new(vec![json!({
            "data": [{ "gid": "user-1", "name": "Alex Chen" }],
            "next_page": null
        })]);
        let client =
            HttpAsanaClient::with_transport(transport, "pat_123", Some("ws_42".to_string()));

        let users = client.list_users().expect("the directory loads");

        assert_eq!(users[0].name.as_deref(), Some("Alex Chen"));
        let requests = client.transport.requests.borrow();
        assert_eq!(requests[0].0, "users");
        assert!(requests[0]
            .1
            .contains(&("workspace".to_string(), "ws_42".to_string())));
    }

    /// Without a workspace the endpoint answers with every user the token can
    /// see anywhere, which is a different question from "who can I assign
    /// this to".
    #[test]
    fn the_user_directory_needs_a_workspace() {
        let client = HttpAsanaClient::with_transport(MockTransport::new(vec![]), "pat_123", None);

        let err = client.list_users().expect_err("no workspace, no directory");

        assert!(matches!(err, Error::Backend(message) if message.contains("workspace_gid")));
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
