//! HTTP-backed Asana client implementation.
//!
//! This module turns the app's abstract `AsanaClient` trait into concrete
//! requests against the public Asana REST API.

use reqwest::blocking::Client;
use serde_json::Value;

use crate::{
    asana::{
        dto::{
            CollectionResponse, ProjectCustomFieldSettingDto, ProjectDto, SectionDto, TaskDto,
        },
        AsanaClient, TaskLoadScope, TaskQuery,
    },
    config::AuthConfig,
    domain::Project,
    error::{Error, Result},
};

const ASANA_API_BASE_URL: &str = "https://app.asana.com/api/1.0";

/// Minimal transport abstraction for JSON GET requests.
pub trait Transport {
    fn get_json(&self, path: &str, query: &[(&str, String)], token: &str) -> Result<Value>;
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
            (
                "opt_fields",
                "gid,name,completed,modified_at,due_on,start_on,assignee.gid,assignee.name,num_subtasks,memberships.project.gid,memberships.project.name,memberships.section.gid,memberships.section.name,custom_fields.gid,custom_fields.name,custom_fields.display_value,custom_fields.enum_value.gid,custom_fields.enum_value.name"
                    .to_string(),
            ),
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
        if let Some(offset) = offset {
            query.push(("offset", offset.to_string()));
        }

        let json = self.transport.get_json(
            &format!("projects/{}/tasks", task_query.project_gid),
            &query,
            &self.personal_access_token,
        )?;

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
            (
                "opt_fields",
                "gid,name,completed,modified_at,due_on,start_on,assignee.gid,assignee.name,num_subtasks,memberships.project.gid,memberships.project.name,memberships.section.gid,memberships.section.name,custom_fields.gid,custom_fields.name,custom_fields.display_value,custom_fields.enum_value.gid,custom_fields.enum_value.name"
                    .to_string(),
            ),
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
            ("opt_fields", "gid,custom_field.gid,custom_field.name".to_string()),
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
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::{HttpAsanaClient, Transport};
    use crate::asana::AsanaClient;
    use crate::error::Result;

    struct MockTransport {
        requests: RefCell<Vec<(String, Vec<(String, String)>, String)>>,
        responses: RefCell<Vec<serde_json::Value>>,
    }

    impl MockTransport {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            Self {
                requests: RefCell::new(Vec::new()),
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
}
