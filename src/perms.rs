//! `perms` module: permission traps.
//!
//! Mode-000 files and directories, write-only files, directories missing
//! the execute bit (stat-able but not traversable) or the read bit
//! (traversable but not listable), and a sticky 1777 directory. The writer
//! applies restrictive directory modes last (deepest first) so the tree can
//! be populated; `verify` temporarily relaxes and restores them; `clean`
//! repairs them before deletion — `rm -rf` alone gets stuck on these.
//!
//! Note: a process running as root bypasses mode bits, so the *traps* only
//! bite unprivileged tools — but generation and verification behave
//! identically either way.

use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "perms";
pub const SUMMARY: &str =
    "mode-000 files and dirs, write-only files, no-exec and no-read dirs, sticky bits";

fn here(name: &str) -> PathBuf {
    PathBuf::from(NAME).join(name)
}

pub fn build(_opts: &Options) -> Vec<Entry> {
    vec![
        Entry::dir(NAME, 0o755),
        // Files with every access bit stripped in a different way.
        Entry::file(here("no-read.txt"), 0o000, Content::Text { len: 64 }),
        Entry::file(here("write-only.txt"), 0o200, Content::Text { len: 64 }),
        Entry::file(here("read-only.txt"), 0o444, Content::Text { len: 64 }),
        Entry::file(here("exec-only.txt"), 0o111, Content::Text { len: 64 }),
        Entry::file(here("wide-open.txt"), 0o777, Content::Text { len: 64 }),
        // A directory nobody can enter — with a file already inside it.
        Entry::dir(here("no-access-dir"), 0o000),
        Entry::file(
            here("no-access-dir/hidden.txt"),
            0o644,
            Content::Text { len: 64 },
        ),
        // rw- but no execute: children can be listed, never stat-ed.
        Entry::dir(here("no-exec-dir"), 0o600),
        Entry::file(
            here("no-exec-dir/unreachable.txt"),
            0o644,
            Content::Text { len: 64 },
        ),
        // -wx but no read: children can be stat-ed by name, never listed.
        Entry::dir(here("no-read-dir"), 0o300),
        Entry::file(
            here("no-read-dir/ghost.txt"),
            0o644,
            Content::Text { len: 64 },
        ),
        // Sticky bit, /tmp-style.
        Entry::dir(here("sticky-dir"), 0o1777),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Kind;

    #[test]
    fn no_access_dir_has_a_child_planned_inside_it() {
        // The trap only works if there is something in there to miss.
        let v = build(&Options::default());
        let dir = v.iter().find(|e| e.path == here("no-access-dir")).unwrap();
        assert_eq!(dir.kind, Kind::Dir { mode: 0o000 });
        assert!(v.iter().any(|e| e.path == here("no-access-dir/hidden.txt")));
    }

    #[test]
    fn restrictive_dirs_come_before_their_children_in_plan_order() {
        // The writer relies on plan order for creation; restrictive chmod
        // happens in a separate final pass.
        let v = build(&Options::default());
        let dir_idx = v
            .iter()
            .position(|e| e.path == here("no-exec-dir"))
            .unwrap();
        let child_idx = v
            .iter()
            .position(|e| e.path == here("no-exec-dir/unreachable.txt"))
            .unwrap();
        assert!(dir_idx < child_idx);
    }

    #[test]
    fn sticky_bit_survives_in_the_planned_mode() {
        let v = build(&Options::default());
        let sticky = v.iter().find(|e| e.path == here("sticky-dir")).unwrap();
        assert_eq!(sticky.kind, Kind::Dir { mode: 0o1777 });
    }

    #[test]
    fn entry_count_is_stable() {
        assert_eq!(build(&Options::default()).len(), 13);
    }
}
