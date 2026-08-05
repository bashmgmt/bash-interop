//! Running bash without writing a rig.
//!
//! A tool that keeps what it hears and has no resources of its own does not
//! need to name a session or an output. These two cover that, over a private
//! [`Rig`] nobody else has to see.

use super::capture::Capture;
use super::error::RigError;
use super::run::{ExitStatus, Rig, Setup, Turn};
use super::wire::{Line, Reply};

/// Runs `argv`, keeping everything and answering nothing.
///
/// A shell that asks anyway gets 127 — the status a shell uses for a word it
/// cannot find, which is what an absent operator amounts to.
pub fn listen(setup: Setup, argv: &[String]) -> Result<(Capture, ExitStatus), RigError> {
    converse(setup, argv, |_seen, _asked| Ok(Reply::status(127)))
}

/// Runs `argv`, keeping everything, answering each question from what has
/// been heard so far.
pub fn converse<A>(
    setup: Setup,
    argv: &[String],
    answer: A,
) -> Result<(Capture, ExitStatus), RigError>
where
    A: FnMut(&Capture, &Turn) -> Result<Reply, RigError>,
{
    Conversing { setup, answer: std::cell::RefCell::new(answer) }.run(argv)
}

/// The answer is `FnMut` but `Rig::answer` takes `&self`, so the closure is
/// borrowed rather than held: one borrow, at one call site, never nested.
struct Conversing<A> {
    setup: Setup,
    answer: std::cell::RefCell<A>,
}

impl<A> Rig for Conversing<A>
where
    A: FnMut(&Capture, &Turn) -> Result<Reply, RigError>,
{
    type Session = Capture;
    type Output = (Capture, ExitStatus);

    fn start(&self) -> Result<(Setup, Capture), RigError> {
        Ok((self.setup.clone(), Capture::default()))
    }

    fn heard(&self, seen: &mut Capture, said: Line) -> Result<(), RigError> {
        seen.lines.push(said);
        Ok(())
    }

    fn answer(&self, seen: &mut Capture, asked: &Turn) -> Result<Reply, RigError> {
        (self.answer.borrow_mut())(seen, asked)
    }

    fn ended(&self, seen: Capture, status: ExitStatus) -> Result<Self::Output, RigError> {
        Ok((seen, status))
    }
}
