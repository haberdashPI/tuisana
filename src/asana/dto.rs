use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CollectionResponse<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub next_page: Option<Page>,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    pub offset: String,
}

#[derive(Debug, Deserialize)]
pub struct ProjectDto {
    pub gid: String,
    pub name: String,
}
