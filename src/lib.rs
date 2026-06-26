//! Library entry point for the Git-native issue tracker CLI.

mod agents;
#[cfg(feature = "beads-import")]
mod beads_import;
mod commands;
pub mod error;
mod model;
mod readiness;
mod store;
mod views;

pub use commands::{Cli, run};
