use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    ConfigParse(toml::de::Error),
    ConfigValidation(String),
    Io(std::io::Error),
    Backend(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ConfigParse(err) => write!(f, "failed to parse config: {err}"),
            Error::ConfigValidation(err) => write!(f, "invalid config: {err}"),
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

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn formats_validation_and_backend_errors() {
        let validation = Error::ConfigValidation("bad version".to_string());
        let backend = Error::Backend("offline".to_string());

        assert_eq!(validation.to_string(), "invalid config: bad version");
        assert_eq!(backend.to_string(), "backend error: offline");
    }

    #[test]
    fn converts_io_and_toml_errors() {
        let io_error = std::io::Error::other("disk full");
        let error: Error = io_error.into();
        assert!(matches!(error, Error::Io(_)));

        let toml_error = toml::from_str::<toml::Value>("=").expect_err("toml should fail");
        let error: Error = toml_error.into();
        assert!(matches!(error, Error::ConfigParse(_)));
    }
}
