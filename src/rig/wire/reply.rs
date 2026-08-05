//! What a blocked shell is told to run next.

use std::path::Path;

use super::message;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reply(Vec<String>);

impl Reply {
    pub fn of(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(words.into_iter().map(Into::into).collect())
    }

    pub fn status(code: i32) -> Self {
        Self::of(["return".to_string(), code.to_string()])
    }

    pub fn source(path: &Path) -> Self {
        Self::of(["source", &path.to_string_lossy()])
    }

    pub fn words(&self) -> &[String] {
        &self.0
    }

    pub(crate) fn to_message(&self) -> String {
        message::literal(&self.0)
    }
}
