#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub starred: bool,
}

impl Project {
    pub fn new(id: impl Into<String>, name: impl Into<String>, starred: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            starred,
        }
    }
}

