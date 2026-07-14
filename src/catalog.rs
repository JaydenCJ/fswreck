//! The module registry: every wreck module, in canonical order.
//!
//! Catalog order — not user selection order — decides plan order, so any
//! subset selection is reproducible regardless of how the flags were typed.

use crate::plan::Options;
use crate::spec::Entry;
use crate::{deep, exotic, names, perms, symlinks, unicode};

pub struct ModuleInfo {
    pub name: &'static str,
    pub summary: &'static str,
    pub build: fn(&Options) -> Vec<Entry>,
}

pub const MODULES: &[ModuleInfo] = &[
    ModuleInfo {
        name: unicode::NAME,
        summary: unicode::SUMMARY,
        build: unicode::build,
    },
    ModuleInfo {
        name: names::NAME,
        summary: names::SUMMARY,
        build: names::build,
    },
    ModuleInfo {
        name: symlinks::NAME,
        summary: symlinks::SUMMARY,
        build: symlinks::build,
    },
    ModuleInfo {
        name: deep::NAME,
        summary: deep::SUMMARY,
        build: deep::build,
    },
    ModuleInfo {
        name: perms::NAME,
        summary: perms::SUMMARY,
        build: perms::build,
    },
    ModuleInfo {
        name: exotic::NAME,
        summary: exotic::SUMMARY,
        build: exotic::build,
    },
];

pub fn all() -> &'static [ModuleInfo] {
    MODULES
}

pub fn all_names() -> Vec<String> {
    MODULES.iter().map(|m| m.name.to_string()).collect()
}

pub fn find(name: &str) -> Option<&'static ModuleInfo> {
    MODULES.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_six_uniquely_named_modules() {
        let names = all_names();
        assert_eq!(names.len(), 6);
        let mut dedup = names.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), names.len(), "duplicate module names");
    }

    #[test]
    fn every_module_prefixes_its_entries_with_its_own_name() {
        // Modules must stay in their own subtree so subsets compose and the
        // manifest stays readable.
        let opts = Options::default();
        for info in all() {
            for e in (info.build)(&opts) {
                assert!(
                    e.path.starts_with(info.name),
                    "{} planned {:?} outside its subtree",
                    info.name,
                    e.path
                );
            }
        }
    }

    #[test]
    fn find_is_exact_match_only() {
        assert!(find("unicode").is_some());
        assert!(find("Unicode").is_none());
        assert!(find("uni").is_none());
    }
}
