//! One error for everything that stops a run.

use std::error::Error;
use std::fmt;

type Cause = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub struct RigError {
    doing: String,
    cause: Cause,
}

impl RigError {
    pub fn new(doing: impl Into<String>, cause: impl Into<Cause>) -> Self {
        Self { doing: doing.into(), cause: cause.into() }
    }
}

impl fmt::Display for RigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.doing, self.cause)
    }
}

impl Error for RigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.cause)
    }
}

pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, RigError>;
}

impl<T, E: Into<Cause>> Doing<T> for Result<T, E> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, RigError> {
        self.map_err(|cause| RigError::new(what(), cause))
    }
}
