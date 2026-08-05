//! The protocol: bytes on a pipe, frames, messages, and the one reply.

mod framing;
mod message;
mod pipes;
mod reply;

pub use framing::{Reassembly, DELIMITER, FRAME_LIMIT};
pub use message::{field, FromRecord, Line, Micros, Pid, Record, Stamp, Stamped, ASK_TAG};
pub use pipes::Wire;
pub use reply::Reply;
