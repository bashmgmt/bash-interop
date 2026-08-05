//! A mechanism: bash definitions and control verbs.
//!
//! Two contribution points, matching the two things a subject can do — speak
//! (call a function that ships a message) and ask (call `BC_INSTR`).
//!
//! [`Codegen`] is the only way to construct a send, and it always splices the
//! owner guard, so writing through another shell's descriptor is impossible
//! by construction rather than by discipline.

use super::control::Verb;
use super::frame::Kind;
use super::src::BashSrc;

pub trait Instrument {
    fn name(&self) -> &str;

    fn bash(&self, codegen: &Codegen) -> BashSrc;

    fn verbs(&self) -> Vec<Verb> {
        Vec::new()
    }
}

#[derive(Clone, Copy, Default)]
pub struct Codegen {
    debug: bool,
}

impl Codegen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn debugging(debug: bool) -> Self {
        Self { debug }
    }

    /// Ships indexed array `$array` as one message, fire and forget.
    pub fn emit(&self, array: &str) -> BashSrc {
        BashSrc::seq([self.guard(), self.send(array, Kind::Post)])
    }

    /// Ships `$array` as a question. The caller blocks afterwards; only
    /// `BC_INSTR` should use this.
    pub fn ask(&self, array: &str) -> BashSrc {
        self.send(array, Kind::Ask)
    }

    /// Sends without the guard, for the one caller that has just run it.
    pub fn post_unguarded(&self, array: &str) -> BashSrc {
        self.send(array, Kind::Post)
    }

    fn guard(&self) -> BashSrc {
        BashSrc::raw("[[ $BASHPID == \"$__BC__owner\" ]] || __BC__join")
    }

    fn send(&self, array: &str, kind: Kind) -> BashSrc {
        let marker = kind.marker();
        let mut lines = vec![BashSrc::raw(format!("__BC__msg=\"(${{{array}[*]@Q}})\""))];
        if self.debug {
            lines.push(BashSrc::raw(format!(
                "__BC__log send {marker} {array} ${{#__BC__msg}}"
            )));
        }
        lines.push(BashSrc::raw(format!(
            "if (( ${{#__BC__msg}} <= __BC__limit )); then\n\
             {I}printf '%s %s %s {marker} %s\\n' \"$EPOCHREALTIME\" \"$BASHPID\" \
             \"$((__BC__seq++))\" \"$__BC__msg\" >&$__BC__up\n\
             else\n\
             {I}__BC__split '{marker}' \"$__BC__msg\"\n\
             fi",
            I = "    ",
        )));
        BashSrc::seq(lines)
    }

    /// `name() { before; "$@"; local rc=$?; after; return "$rc"; }` — the
    /// continuation's status is preserved by construction, and `$?` is read
    /// as the first statement after it, which is the one thing easy to get
    /// wrong in a `continuation == "$@"` framework.
    pub fn cps_wrapper(&self, name: &str, before: BashSrc, after: BashSrc) -> BashSrc {
        BashSrc::func(
            name,
            BashSrc::seq([
                before,
                BashSrc::raw("\"$@\""),
                BashSrc::raw("local rc=$?"),
                after,
                BashSrc::raw("return \"$rc\""),
            ]),
        )
    }
}
