//! Reusable xtask helpers for Godot + Rust (gdext) projects.
//!
//! The crate is intentionally a library: consuming projects keep their own thin
//! `xtask` binary and forward project-specific commands into these modules.

pub mod paths;

#[cfg(feature = "process")]
pub mod process;

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "godot")]
pub mod godot;

#[cfg(feature = "gdext")]
pub mod update;

#[cfg(feature = "addons")]
pub mod addons;

#[cfg(feature = "run-export")]
pub mod export;

#[cfg(feature = "run-export")]
pub mod run;

#[cfg(feature = "files")]
pub mod files;

#[cfg(feature = "isolation")]
pub mod isolation;
