//! `exotic` module: file types and shapes beyond "regular file in a folder".
//!
//! A FIFO (naive readers `open()` it and block forever), a hardlink pair
//! (dedup-unaware tools double their byte counts), a 1 MiB sparse file that
//! occupies almost no disk, an empty file, an empty directory (many VCS and
//! sync tools silently drop these), and a 128-file directory for readdir
//! ordering assumptions.

use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "exotic";
pub const SUMMARY: &str =
    "FIFO, hardlink pair, 1MiB sparse file, empty file/dir, 128-file wide dir";

/// Logical size of the sparse file. Allocated blocks stay near zero.
pub const SPARSE_LEN: u64 = 1 << 20;

/// Number of files in the `wide/` directory.
pub const WIDE_COUNT: u32 = 128;

fn here(name: &str) -> PathBuf {
    PathBuf::from(NAME).join(name)
}

pub fn build(_opts: &Options) -> Vec<Entry> {
    let mut v = vec![
        Entry::dir(NAME, 0o755),
        Entry::file(here("empty.txt"), 0o644, Content::Empty),
        Entry::dir(here("empty-dir"), 0o755),
        Entry::file(
            here("sparse.bin"),
            0o644,
            Content::Sparse { len: SPARSE_LEN },
        ),
        Entry::file(
            here("hardlink-original.txt"),
            0o644,
            Content::Text { len: 128 },
        ),
        Entry::hardlink(here("hardlink-copy.txt"), here("hardlink-original.txt")),
        Entry::fifo(here("fifo.pipe"), 0o644),
        // Extension edge cases.
        Entry::file(
            here("archive.tar.gz.bak.old.1"),
            0o644,
            Content::Bytes { len: 256 },
        ),
        Entry::file(here("no-extension"), 0o644, Content::Bytes { len: 64 }),
        Entry::file(here(".hidden"), 0o644, Content::Text { len: 64 }),
    ];
    v.push(Entry::dir(here("wide"), 0o755));
    for i in 0..WIDE_COUNT {
        v.push(Entry::file(
            here(&format!("wide/f{i:03}")),
            0o644,
            Content::Empty,
        ));
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
    fn hardlink_copy_references_the_original() {
        let v = entries();
        let link = v
            .iter()
            .find(|e| e.path == here("hardlink-copy.txt"))
            .unwrap();
        assert_eq!(
            link.kind,
            Kind::Hardlink {
                original: here("hardlink-original.txt")
            }
        );
    }

    #[test]
    fn includes_a_mebibyte_sparse_file_a_fifo_and_an_empty_dir() {
        let v = entries();
        let sparse = v.iter().find(|e| e.path == here("sparse.bin")).unwrap();
        match &sparse.kind {
            Kind::File { content, .. } => assert_eq!(content.len(), 1_048_576),
            other => panic!("expected file, got {other:?}"),
        }
        assert!(v.iter().any(|e| matches!(e.kind, Kind::Fifo { .. })));
        assert!(v
            .iter()
            .any(|e| e.path == here("empty-dir") && matches!(e.kind, Kind::Dir { .. })));
    }

    #[test]
    fn wide_dir_has_the_advertised_file_count() {
        let v = entries();
        let wide = v
            .iter()
            .filter(|e| e.path.starts_with(here("wide")) && matches!(e.kind, Kind::File { .. }))
            .count();
        assert_eq!(wide, WIDE_COUNT as usize);
    }

    #[test]
    fn entry_count_is_stable() {
        assert_eq!(entries().len(), 11 + WIDE_COUNT as usize);
    }
}
