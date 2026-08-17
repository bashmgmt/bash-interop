//! The book's Rust, compiled. Every fenced Rust block in `docs/` that is
//! not the tree's own code is anchored here, and `docs/sync-quotes.bash`
//! keeps the fences identical to these regions. Compiling is the
//! assertion — nothing runs.
#![allow(dead_code)]

use std::ffi::OsString;

use bash_interop::rig::{Failure, Layout, Provision};

fn environment<E: FnOnce(&Layout) -> Result<Vec<(OsString, OsString)>, Failure>>(_: E) {}

// ANCHOR: deploy-join
/// The standard initiation — data the run's closure hands to `bash_env`;
/// run only where a client or a provisioned file says so.
fn deploy_join(at: &Layout) -> String {
    format!("BC_JOIN DEPLOY {}\n", bash_strings::emit_scalar(at.text()))
}
// ANCHOR_END: deploy-join

/// docs/driving.md — the three usual environment closures.
fn the_three_usual_sentences() {
    environment(
        // ANCHOR: env-joining
        // Blanket: provision a joining startup file. Every non-interactive
        // bash in the subject's tree joins as it starts — the right default
        // for subjects that know nothing of the session. The line is the
        // wrapper's own statement (rigs.md: the sketch's deploy_join).
        |at| Ok(vec![at.bash_env(Provision::Joining(&deploy_join(at)))?]),
        // ANCHOR_END: env-joining
    );
    environment(
        // ANCHOR: env-definitions
        // Chosen: provision definitions only, and hand the coordinate to
        // the scripts under a name of YOUR convention — they initiate where
        // they say. (bashprof spells this BASHPROF_SESSION, bashcap
        // BASHCAP_SESSION.)
        |at| {
            Ok(vec![
                at.bash_env(Provision::Definitions)?,
                ("DEPLOY_SESSION".into(), at.text().into()),
            ])
        },
        // ANCHOR_END: env-definitions
    );
    environment(
        // ANCHOR: env-nothing
        // Nothing: the subject runs with no additions at all. Shells can
        // still join by hand if some script knows the workspace by other
        // means.
        |_at| Ok(vec![]),
        // ANCHOR_END: env-nothing
    );
}
