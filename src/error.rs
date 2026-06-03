use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    ConfigParse(toml::de::Error),
    Io(std::io::Error),
    Backend(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ConfigParse(err) => write!(f, "failed to parse config: {err}"),
            Error::Io(err) => write!(f, "io error: {err}"),
            Error::Backend(err) => write!(f, "backend error: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<toml::de::Error> for Error {
    fn from(value: toml::de::Error) -> Self {
        Self::ConfigParse(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

