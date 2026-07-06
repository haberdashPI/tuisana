//! Project records and visibility state.
//!
//! Projects are the top-level units shown in the project list. The app keeps
//! project visibility alongside the project record itself so the UI can render
//! hidden state without needing a separate map.

/// Distinguishes a real Asana project from the synthetic "assigned to me" row.
///
/// Code that needs to treat the assigned-to-me row differently (sorting,
/// persistence, opening in a browser, query building) should match on this
/// instead of comparing `Project.id` against a magic constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    Normal,
    AssignedToMe,
}

/// A project known to the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// Stable Asana project id.
    pub id: String,
    /// Display name shown in the UI.
    pub name: String,
    /// Whether the project is starred in Asana.
    pub starred: bool,
    /// Whether the project is hidden in the local UI.
    pub hidden: bool,
    /// Whether this is a real Asana project or the assigned-to-me pseudo-project.
    pub kind: ProjectKind,
}

impl Project {
    /// Builds a visible project with the given id, name, and star flag.
    pub fn new(id: impl Into<String>, name: impl Into<String>, starred: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            starred,
            hidden: false,
            kind: ProjectKind::Normal,
        }
    }

    /// Builds a project with explicit visibility metadata.
    pub fn with_visibility(
        id: impl Into<String>,
        name: impl Into<String>,
        starred: bool,
        hidden: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            starred,
            hidden,
            kind: ProjectKind::Normal,
        }
    }

    /// Builds the synthetic "No Project (Assigned to Me)" row for the given
    /// Asana user gid (typically resolved once at startup via the
    /// `AsanaClient::current_user_gid` login lookup, not configured by hand).
    pub fn assigned_to_me(user_gid: impl Into<String>) -> Self {
        Self {
            id: user_gid.into(),
            name: "No Project (Assigned to Me)".to_string(),
            starred: false,
            hidden: false,
            kind: ProjectKind::AssignedToMe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Project, ProjectKind};

    #[test]
    fn assigned_to_me_builds_a_visible_unstarred_pseudo_project() {
        let project = Project::assigned_to_me("user_1");

        assert_eq!(project.id, "user_1");
        assert_eq!(project.name, "No Project (Assigned to Me)");
        assert!(!project.starred);
        assert!(!project.hidden);
        assert_eq!(project.kind, ProjectKind::AssignedToMe);
    }

    #[test]
    fn new_and_with_visibility_default_to_normal_kind() {
        assert_eq!(Project::new("1", "Inbox", true).kind, ProjectKind::Normal);
        assert_eq!(
            Project::with_visibility("1", "Inbox", true, true).kind,
            ProjectKind::Normal
        );
    }
}
