//! The bash every participating shell sources.

use std::path::Path;

use super::BashSrc;
use crate::bash::rig::error::RigError;
use crate::bash::rig::wire::FRAME_LIMIT;

const WIRE: &str = include_str!("wire.bash");

pub fn prelude(bash: &BashSrc, debug: bool, dir: &Path, up: &Path) -> Result<BashSrc, RigError> {
    let quote = |path: &Path| crate::bash::value::emit_scalar(&path.to_string_lossy());

    Ok(BashSrc::seq([
        BashSrc::raw(format!("__BC__UP={}", quote(up))),
        BashSrc::raw(format!("__BC__DIR={}", quote(dir))),
        BashSrc::raw(format!("__BC__limit={FRAME_LIMIT}")),
        BashSrc::raw(format!("__BC__DEBUG={}", if debug { "1" } else { "" })),
        BashSrc::raw(WIRE),
        bash.clone(),
    ]))
}
