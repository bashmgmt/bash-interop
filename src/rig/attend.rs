//! One shell, start to finish, as a task of its own.

use std::sync::Arc;

use tokio::sync::watch;

use super::wire::{Line, Micros, Pipe, Verb};
use super::{Attended, Message, Reacting, Shell};
use crate::failure::Failure;

/// Serve one shell until nobody can write on its pipe, or until the session
/// closes — whichever comes first — then finish its reaction.
pub(super) async fn attend<A: Reacting>(
    shell: Arc<Shell>,
    mut pipe: Pipe,
    mut reaction: A,
    mut closing: watch::Receiver<bool>,
) -> Result<Attended<A::Kept>, Failure> {
    let parted = loop {
        tokio::select! {
            biased;
            next = pipe.lines.next() => match next? {
                Some(line) => react(&mut reaction, &pipe, line).await?,
                None => break Some(Micros::now()),
            },
            _ = closing.changed() => {
                for line in pipe.lines.drain()? {
                    react(&mut reaction, &pipe, line).await?;
                }
                break None;
            }
        }
    };
    pipe.close()?;

    Ok(Attended { shell, kept: reaction.finish().await?, parted })
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
