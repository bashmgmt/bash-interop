//! How bash ended.

use std::fmt;

/// How bash ended. `wait(2)` yields exactly one of these, and both fields are
/// the width the kernel gives them.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExitStatus {
    Code(u8),
    Signal(u8),
}

impl ExitStatus {
    /// What a shell would report for it: `128 + n` for a signal.
    pub fn shell_code(self) -> i32 {
        match self {
            Self::Code(code) => i32::from(code),
            Self::Signal(signal) => 128 + i32::from(signal),
        }
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit {code}"),
            Self::Signal(signal) => write!(f, "killed by signal {signal}"),
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    /// After `wait(2)` a process has either exited or been signalled, so
    /// reading the two fields out of the raw status is total: `WTERMSIG` is
    /// the low seven bits, `WEXITSTATUS` the second byte, and there is no
    /// third outcome to default to.
    fn from(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;

        let raw = status.into_raw();
        match status.signal() {
            Some(_) => Self::Signal((raw & 0x7f) as u8),
            None => Self::Code(((raw >> 8) & 0xff) as u8),
        }
    }
}
