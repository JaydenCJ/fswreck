//! fswreck — deterministically generates adversarial file trees.
//!
//! The library is organized as pure, unit-testable layers:
//!
//! - [`rng`] / [`pathcodec`] / [`json`] — deterministic primitives (PRNG,
//!   FNV-1a hashing, lossless path encoding, a minimal JSON parser).
//! - [`spec`] — the data model: an [`spec::Entry`] per planned node.
//! - the wreck modules ([`unicode`], [`names`], [`symlinks`], [`deep`],
//!   [`perms`], [`exotic`]) — each returns a curated `Vec<Entry>`.
//! - [`plan`] / [`catalog`] — module registry and plan assembly.
//! - [`manifest`] — the on-disk JSON manifest (write + parse).
//! - [`writer`] / [`verify`] / [`clean`] — the only layers that touch disk.
//! - [`cli`] — argument parsing and command dispatch.

#[cfg(not(unix))]
compile_error!("fswreck targets Unix-like systems (Linux, macOS): the fixtures it generates (invalid UTF-8 names, FIFOs, mode bits) do not exist elsewhere");

pub mod catalog;
pub mod clean;
pub mod cli;
pub mod deep;
pub mod exotic;
pub mod json;
pub mod manifest;
pub mod names;
pub mod pathcodec;
pub mod perms;
pub mod plan;
pub mod rng;
pub mod spec;
pub mod symlinks;
pub mod unicode;
pub mod verify;
pub mod writer;
