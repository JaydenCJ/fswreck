//! Plan assembly: turn a module selection into one ordered `Vec<Entry>`.
//!
//! The plan is the single source of truth — the writer materializes it, the
//! manifest records it, and `verify` replays it. Invariants enforced here:
//! no duplicate paths, every parent directory precedes its children, and
//! hardlink originals precede their links.

use std::collections::HashSet;
use std::path::Path;

use crate::catalog;
use crate::spec::{Entry, Kind};

/// Generation options shared by every module.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Seed for all file contents (names are curated and seed-independent).
    pub seed: u64,
    /// Nesting depth used by the `deep` module.
    pub depth: u32,
    /// Enabled module names, in catalog order.
    pub modules: Vec<String>,
}

pub const DEFAULT_SEED: u64 = 42;
pub const DEFAULT_DEPTH: u32 = 32;

impl Default for Options {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            depth: DEFAULT_DEPTH,
            modules: catalog::all_names(),
        }
    }
}

/// Build the full plan for `opts`. Modules always run in catalog order, so
/// `--modules names,unicode` and `--modules unicode,names` produce the same
/// tree.
pub fn build_plan(opts: &Options) -> Result<Vec<Entry>, String> {
    let mut seen_names: HashSet<&str> = HashSet::new();
    for name in &opts.modules {
        if catalog::find(name).is_none() {
            return Err(format!(
                "unknown module {:?} (available: {})",
                name,
                catalog::all_names().join(", ")
            ));
        }
        if !seen_names.insert(name.as_str()) {
            return Err(format!("module {name:?} listed twice"));
        }
    }
    if opts.depth == 0 || opts.depth > 512 {
        return Err(format!(
            "--depth must be between 1 and 512, got {}",
            opts.depth
        ));
    }

    let mut entries = Vec::new();
    for info in catalog::all() {
        if opts.modules.iter().any(|m| m == info.name) {
            entries.extend((info.build)(opts));
        }
    }
    check_invariants(&entries)?;
    Ok(entries)
}

fn check_invariants(entries: &[Entry]) -> Result<(), String> {
    let mut seen: HashSet<&Path> = HashSet::new();
    for e in entries {
        if !seen.insert(e.path.as_path()) {
            return Err(format!(
                "internal error: duplicate planned path {:?}",
                e.path
            ));
        }
        if let Some(parent) = e.path.parent() {
            if !parent.as_os_str().is_empty() && !seen.contains(parent) {
                return Err(format!(
                    "internal error: {:?} planned before its parent directory",
                    e.path
                ));
            }
        }
        if let Kind::Hardlink { original } = &e.kind {
            if !seen.contains(original.as_path()) {
                return Err(format!(
                    "internal error: hardlink {:?} planned before its original",
                    e.path
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathcodec;

    #[test]
    fn default_plan_includes_every_module_and_passes_invariants() {
        let plan = build_plan(&Options::default()).unwrap();
        for info in catalog::all() {
            let prefix = Path::new(info.name);
            assert!(
                plan.iter().any(|e| e.path.starts_with(prefix)),
                "module {} contributed no entries",
                info.name
            );
        }
    }

    #[test]
    fn building_twice_yields_identical_plans_and_seeds_never_move_paths() {
        let a = build_plan(&Options::default()).unwrap();
        let b = build_plan(&Options::default()).unwrap();
        assert_eq!(a, b);
        // A different seed changes file bytes, never the topology.
        let opts = Options {
            seed: 999,
            ..Options::default()
        };
        let c = build_plan(&opts).unwrap();
        let paths = |p: &[Entry]| p.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&a), paths(&c));
    }

    #[test]
    fn subsets_compose_in_catalog_order_regardless_of_flag_order() {
        let o1 = Options {
            modules: vec!["names".into(), "unicode".into()],
            ..Options::default()
        };
        let o2 = Options {
            modules: vec!["unicode".into(), "names".into()],
            ..Options::default()
        };
        assert_eq!(build_plan(&o1).unwrap(), build_plan(&o2).unwrap());
        // And a subset never leaks entries from other modules.
        let only = Options {
            modules: vec!["perms".into()],
            ..Options::default()
        };
        let plan = build_plan(&only).unwrap();
        assert!(!plan.is_empty());
        assert!(plan.iter().all(|e| e.path.starts_with("perms")));
    }

    #[test]
    fn unknown_and_duplicate_modules_are_rejected() {
        let mut opts = Options {
            modules: vec!["nope".into()],
            ..Options::default()
        };
        let err = build_plan(&opts).unwrap_err();
        assert!(err.contains("unknown module"), "{err}");
        assert!(
            err.contains("unicode"),
            "error should list valid modules: {err}"
        );

        opts.modules = vec!["perms".into(), "perms".into()];
        assert!(build_plan(&opts).unwrap_err().contains("twice"));
    }

    #[test]
    fn depth_bounds_are_enforced() {
        let mut opts = Options {
            depth: 0,
            ..Options::default()
        };
        assert!(build_plan(&opts).is_err());
        opts.depth = 513;
        assert!(build_plan(&opts).is_err());
    }

    #[test]
    fn every_planned_path_is_relative_and_round_trips_through_the_codec() {
        // Property over the whole curated set: encode → decode is lossless
        // and never produces a traversal-shaped path.
        for e in build_plan(&Options::default()).unwrap() {
            assert!(e.path.is_relative(), "{:?} is absolute", e.path);
            let enc = pathcodec::encode(&e.path);
            assert!(enc.is_ascii());
            let dec =
                pathcodec::decode_rel(&enc).unwrap_or_else(|err| panic!("{:?}: {err}", e.path));
            assert_eq!(dec, e.path);
        }
    }
}
