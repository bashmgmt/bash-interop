//! The protocol: bytes on a pipe, frames, messages, replies.

mod framing;
mod message;
mod pipes;
mod reply;

pub use framing::{Reassembly, FRAME_LIMIT};
pub use message::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, ASK_TAG};
pub use pipes::Wire;
pub use reply::Reply;
