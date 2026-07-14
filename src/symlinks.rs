//! `symlinks` module: cycles, dangling links, escapes, and an ELOOP chain.
//!
//! Traversal code that follows links naively will loop forever on the
//! self-link or the ping/pong pair, walk out of the fixture through the
//! escape links, or hit `ELOOP` on the 50-link chain (the Linux resolver
//! gives up after 40). fswreck itself never follows a link — every check is
//! `lstat`/`readlink`.

use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "symlinks";
pub const SUMMARY: &str =
    "self-loops, A<->B cycles, dangling/absolute/escaping targets, 50-link ELOOP chain";

/// Length of the link chain. Longer than Linux's 40-hop resolver limit, so
/// opening `chain00` fails with ELOOP even though every link exists.
pub const CHAIN_LEN: u32 = 50;

fn here(name: &str) -> PathBuf {
    PathBuf::from(NAME).join(name)
}

pub fn build(_opts: &Options) -> Vec<Entry> {
    let mut v = vec![
        Entry::dir(NAME, 0o755),
        // A real file so some links have a valid destination.
        Entry::file(here("payload.txt"), 0o644, Content::Text { len: 64 }),
        // Degenerate loops.
        Entry::symlink(here("self"), "self"),
        Entry::symlink(here("ping"), "pong"),
        Entry::symlink(here("pong"), "ping"),
        // Dangling relative, dangling absolute, and a link that points above
        // the fixture root (backup tools must not follow it out).
        Entry::symlink(here("dangling"), "does-not-exist"),
        Entry::symlink(here("absolute"), "/nonexistent/fswreck-target"),
        Entry::symlink(here("escape"), "../../outside-the-fixture"),
        // Healthy links, including one whose own name needs quoting.
        Entry::symlink(here("to-file"), "payload.txt"),
        Entry::symlink(here("link with space"), "payload.txt"),
        // A directory that links back to itself and to its parent: the
        // classic infinite-descent trap for recursive copies.
        Entry::dir(here("loop"), 0o755),
        Entry::symlink(here("loop/up"), ".."),
        Entry::symlink(here("loop/around"), "../loop"),
        Entry::symlink(here("to-dir"), "loop"),
    ];

    // chain00 -> chain01 -> ... -> chain49 -> payload.txt
    for i in 0..CHAIN_LEN {
        let target = if i + 1 == CHAIN_LEN {
            "payload.txt".to_string()
        } else {
            format!("chain{:02}", i + 1)
        };
        v.push(Entry::symlink(here(&format!("chain{i:02}")), target));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Kind;

    fn entries() -> Vec<Entry> {
        build(&Options::default())
    }

    #[test]
    fn cycles_are_planned_as_self_loop_and_ping_pong_pair() {
        let v = entries();
        let e = v.iter().find(|e| e.path == here("self")).unwrap();
        assert_eq!(
            e.kind,
            Kind::Symlink {
                target: "self".into()
            }
        );
        let ping = v.iter().find(|e| e.path == here("ping")).unwrap();
        let pong = v.iter().find(|e| e.path == here("pong")).unwrap();
        assert_eq!(
            ping.kind,
            Kind::Symlink {
                target: "pong".into()
            }
        );
        assert_eq!(
            pong.kind,
            Kind::Symlink {
                target: "ping".into()
            }
        );
    }

    #[test]
    fn chain_is_longer_than_the_kernel_resolver_limit() {
        let v = entries();
        let chain: Vec<_> = v
            .iter()
            .filter(|e| e.path.to_string_lossy().contains("chain"))
            .collect();
        assert_eq!(chain.len(), CHAIN_LEN as usize);
        // The chain must exceed Linux's 40-hop resolver limit by contract.
        const _: () = assert!(CHAIN_LEN > 40);
        // The last hop lands on the payload.
        let last = v
            .iter()
            .find(|e| e.path == here(&format!("chain{:02}", CHAIN_LEN - 1)))
            .unwrap();
        assert_eq!(
            last.kind,
            Kind::Symlink {
                target: "payload.txt".into()
            }
        );
    }

    #[test]
    fn escape_targets_leave_the_fixture_but_only_as_link_text() {
        let v = entries();
        let escape = v.iter().find(|e| e.path == here("escape")).unwrap();
        match &escape.kind {
            Kind::Symlink { target } => assert!(target.starts_with("..")),
            other => panic!("expected symlink, got {other:?}"),
        }
    }

    #[test]
    fn entry_count_is_stable() {
        assert_eq!(entries().len(), 14 + CHAIN_LEN as usize);
    }
}
