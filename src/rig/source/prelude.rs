//! The bash every participating shell sources.

use std::path::Path;

use super::{Asset, BashSrc};
use crate::bash::rig::error::RigError;
use crate::bash::rig::wire::FRAME_LIMIT;

const WIRE: Asset = Asset::new("rig/wire.bash");

/// Folded in a fixed order: the configuration as literals, `wire.bash`, then
/// the tool's own bash. Self-reliant, so `BASH_ENV` is enough and nothing has
/// to be inherited.
pub fn prelude(bash: &BashSrc, debug: bool, dir: &Path, up: &Path) -> Result<BashSrc, RigError> {
    let quote = |path: &Path| crate::bash::value::emit_scalar(&path.to_string_lossy());

    Ok(BashSrc::seq([
        BashSrc::raw(format!("__BC__UP={}", quote(up))),
        BashSrc::raw(format!("__BC__DIR={}", quote(dir))),
        BashSrc::raw(format!("__BC__limit={FRAME_LIMIT}")),
        BashSrc::raw(format!("__BC__DEBUG={}", if debug { "1" } else { "" })),
        WIRE.read()?,
        bash.clone(),
    ]))
}
