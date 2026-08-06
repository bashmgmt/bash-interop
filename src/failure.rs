//! One error for anything that stops a piece of work: what was being done,
//! and what went wrong underneath.

use std::error::Error;
use std::fmt;

type Cause = Box<dyn Error + Send + Sync>;

/// A context and a cause rather than an enum, since every use is `Display`
/// or `source()`.
#[derive(Debug)]
pub struct Failure {
    doing: String,
    cause: Cause,
}

impl Failure {
    pub fn new(doing: impl Into<String>, cause: impl Into<Cause>) -> Self {
        Self { doing: doing.into(), cause: cause.into() }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.doing, self.cause)
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.cause)
    }
}

/// Says what was being attempted when a `Result` went wrong.
pub trait Doing<T> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure>;
}

impl<T, E: Into<Cause>> Doing<T> for Result<T, E> {
    fn doing(self, what: impl FnOnce() -> String) -> Result<T, Failure> {
        self.map_err(|cause| Failure::new(what(), cause))
    }
}
