//! One shell, start to finish, as a task of its own.

use std::sync::Arc;

use tokio::sync::watch;

use super::wire::{Line, Micros, Pipe, Verb};
use super::{Attended, Message, Reacting, Shell};
use crate::failure::Failure;

/// What a task hands back: the shell attended, and a line its pipe was left
/// holding, if any — reported beside what the shell said, not instead of it.
pub(super) struct Attendance<K> {
    pub attended: Attended<K>,
    pub cut: Option<Failure>,
}

/// Serve one shell until nobody can write on its pipe, or until the session
/// closes — whichever comes first — then finish its reaction.
pub(super) async fn attend<A: Reacting>(
    shell: Arc<Shell>,
    mut pipe: Pipe,
    mut reaction: A,
    mut closing: watch::Receiver<bool>,
) -> Result<Attendance<A::Kept>, Failure> {
    let parted = loop {
        tokio::select! {
            biased;
            next = pipe.next() => match next? {
                Some(line) => react(&mut reaction, &pipe, line).await?,
                None => break Some(Micros::now()?),
            },
            _ = closing.changed() => {
                for line in pipe.drain()? {
                    react(&mut reaction, &pipe, line).await?;
                }
                break None;
            }
        }
    };
    let kept = reaction.finish().await?;

    Ok(Attendance {
        attended: Attended {
            shell,
            kept,
            parted,
        },
        cut: pipe.close().err(),
    })
}

async fn react<A: Reacting>(reaction: &mut A, pipe: &Pipe, line: Line) -> Result<(), Failure> {
    let message = Message::read(line)?;

    match message.verb {
        Verb::Say => reaction.hear(message).await,
        Verb::Ask => {
            let answer = reaction.answer(message).await?;

            pipe.answer(answer).await
        }
    }
}
