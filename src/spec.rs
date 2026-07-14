//! The data model: one [`Entry`] per planned node of a hostile tree, plus
//! deterministic content generation.
//!
//! Entries are pure data — nothing in this module touches the filesystem.
//! File bytes are derived from `global_seed XOR fnv1a64(relative_path)`, so a
//! file's content depends only on the seed and its own path, never on
//! generation order or on which modules are enabled alongside it.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::pathcodec;
use crate::rng::{fnv1a64, Fnv64, SplitMix64};

/// What bytes a planned file holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Zero bytes.
    Empty,
    /// Seeded printable ASCII, newline every 64th byte.
    Text { len: u64 },
    /// Seeded raw bytes (full 0–255 range).
    Bytes { len: u64 },
    /// A hole: `len - 1` zero bytes never written to disk, one real byte at
    /// the end. Logical size is large, allocated blocks are tiny.
    Sparse { len: u64 },
}

impl Content {
    pub fn len(&self) -> u64 {
        match self {
            Content::Empty => 0,
            Content::Text { len } | Content::Bytes { len } | Content::Sparse { len } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Dir { mode: u32 },
    File { mode: u32, content: Content },
    Symlink { target: PathBuf },
    Hardlink { original: PathBuf },
    Fifo { mode: u32 },
}

/// One planned node. `path` is always relative to the fixture root.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
}

impl Entry {
    pub fn dir(path: impl Into<PathBuf>, mode: u32) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Dir { mode },
        }
    }

    pub fn file(path: impl Into<PathBuf>, mode: u32, content: Content) -> Self {
        Self {
            path: path.into(),
            kind: Kind::File { mode, content },
        }
    }

    pub fn symlink(path: impl Into<PathBuf>, target: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Symlink {
                target: target.into(),
            },
        }
    }

    pub fn hardlink(path: impl Into<PathBuf>, original: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Hardlink {
                original: original.into(),
            },
        }
    }

    pub fn fifo(path: impl Into<PathBuf>, mode: u32) -> Self {
        Self {
            path: path.into(),
            kind: Kind::Fifo { mode },
        }
    }

    /// Human-readable kind tag, matching the manifest `kind` field.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Dir { .. } => "dir",
            Kind::File { .. } => "file",
            Kind::Symlink { .. } => "symlink",
            Kind::Hardlink { .. } => "hardlink",
            Kind::Fifo { .. } => "fifo",
        }
    }
}

/// The per-file RNG: seeded from the global seed and the file's own path.
pub fn file_rng(seed: u64, rel: &Path) -> SplitMix64 {
    SplitMix64::new(seed ^ fnv1a64(pathcodec::path_bytes(rel)))
}

const TEXT_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789 ";

/// Stream the deterministic bytes of `content` into `w`.
pub fn write_content<W: Write>(
    w: &mut W,
    rng: &mut SplitMix64,
    content: &Content,
) -> io::Result<()> {
    match content {
        Content::Empty => Ok(()),
        Content::Text { len } => {
            let mut buf = Vec::with_capacity((*len).min(8192) as usize);
            for i in 0..*len {
                let b = if i % 64 == 63 {
                    b'\n'
                } else {
                    TEXT_CHARS[(rng.next_u64() % TEXT_CHARS.len() as u64) as usize]
                };
                buf.push(b);
                if buf.len() == 8192 {
                    w.write_all(&buf)?;
                    buf.clear();
                }
            }
            w.write_all(&buf)
        }
        Content::Bytes { len } => {
            let mut left = *len;
            let mut buf = [0u8; 8192];
            while left > 0 {
                let n = left.min(8192) as usize;
                rng.fill(&mut buf[..n]);
                w.write_all(&buf[..n])?;
                left -= n as u64;
            }
            Ok(())
        }
        Content::Sparse { len } => {
            // The logical byte stream: len-1 zeros then a single marker byte.
            let zeros = [0u8; 8192];
            let mut left = len.saturating_sub(1);
            while left > 0 {
                let n = left.min(8192) as usize;
                w.write_all(&zeros[..n])?;
                left -= n as u64;
            }
            if *len > 0 {
                w.write_all(b"S")?;
            }
            Ok(())
        }
    }
}

/// FNV-1a 64 fingerprint of a file's deterministic content.
pub fn content_hash(seed: u64, rel: &Path, content: &Content) -> u64 {
    let mut rng = file_rng(seed, rel);
    let mut hasher = Fnv64::new();
    write_content(&mut hasher, &mut rng, content).expect("hashing cannot fail");
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::FNV_OFFSET;

    #[test]
    fn content_len_reports_logical_sizes_and_empty_hashes_to_the_basis() {
        assert_eq!(Content::Empty.len(), 0);
        assert_eq!(Content::Text { len: 96 }.len(), 96);
        assert_eq!(Content::Sparse { len: 1 << 20 }.len(), 1 << 20);
        assert_eq!(
            content_hash(42, Path::new("x"), &Content::Empty),
            FNV_OFFSET
        );
    }

    #[test]
    fn content_is_keyed_by_seed_and_path() {
        // Same seed + path: identical bytes. Different path under one seed:
        // different bytes — reordering modules can never shuffle which bytes
        // land in which file.
        let c = Content::Text { len: 256 };
        let p = Path::new("a/b.txt");
        let mut out1 = Vec::new();
        let mut out2 = Vec::new();
        write_content(&mut out1, &mut file_rng(42, p), &c).unwrap();
        write_content(&mut out2, &mut file_rng(42, p), &c).unwrap();
        assert_eq!(out1, out2);
        assert_eq!(out1.len(), 256);
        let mut other = Vec::new();
        write_content(&mut other, &mut file_rng(42, Path::new("z")), &c).unwrap();
        assert_ne!(out1, other);
    }

    #[test]
    fn text_content_is_printable_with_terminating_newlines() {
        let mut out = Vec::new();
        write_content(
            &mut out,
            &mut file_rng(1, Path::new("t")),
            &Content::Text { len: 128 },
        )
        .unwrap();
        assert_eq!(out[63], b'\n');
        assert_eq!(out[127], b'\n');
        assert!(out.iter().all(|&b| b == b'\n' || TEXT_CHARS.contains(&b)));
    }

    #[test]
    fn sparse_stream_is_zeros_with_a_final_marker() {
        let mut out = Vec::new();
        write_content(
            &mut out,
            &mut file_rng(1, Path::new("s")),
            &Content::Sparse { len: 100 },
        )
        .unwrap();
        assert_eq!(out.len(), 100);
        assert!(out[..99].iter().all(|&b| b == 0));
        assert_eq!(out[99], b'S');
    }

    #[test]
    fn content_hash_streams_the_full_logical_length() {
        let h1 = content_hash(1, Path::new("s"), &Content::Sparse { len: 100 });
        let h2 = content_hash(1, Path::new("s"), &Content::Sparse { len: 101 });
        assert_ne!(h1, h2, "hash must cover every logical byte");
    }
}
