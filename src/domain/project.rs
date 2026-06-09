//! Project records and visibility state.
//!
//! Projects are the top-level units shown in the project list. The app keeps
//! project visibility alongside the project record itself so the UI can render
//! hidden state without needing a separate map.

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
}

impl Project {
    /// Builds a visible project with the given id, name, and star flag.
    pub fn new(id: impl Into<String>, name: impl Into<String>, starred: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            starred,
            hidden: false,
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
        }
    }
}
