//! Materialize a plan on disk.
//!
//! Ordering is the whole trick:
//! 1. directories (plan order — parents first),
//! 2. regular files with their seeded content,
//! 3. FIFOs, then hardlinks, then symlinks,
//! 4. permission bits: files first, then directories *deepest first*, so a
//!    mode-000 directory is locked only after everything inside it exists.
//!
//! Every mode is applied explicitly with `chmod`, so the resulting tree is
//! identical regardless of the caller's umask.

use std::fs;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;

use crate::spec::{file_rng, write_content, Content, Entry, Kind};

/// Write `entries` under `root`, creating `root` if needed.
pub fn write_tree(root: &Path, seed: u64, entries: &[Entry]) -> io::Result<()> {
    fs::create_dir_all(root)?;

    // Pass 1: directories.
    for e in entries {
        if let Kind::Dir { .. } = e.kind {
            fs::create_dir(root.join(&e.path))?;
        }
    }

    // Pass 2: regular files.
    for e in entries {
        if let Kind::File { content, .. } = &e.kind {
            write_file(&root.join(&e.path), seed, &e.path, content)?;
        }
    }

    // Pass 3: FIFOs, hardlinks, symlinks.
    for e in entries {
        match &e.kind {
            Kind::Fifo { .. } => mkfifo(&root.join(&e.path))?,
            Kind::Hardlink { original } => fs::hard_link(root.join(original), root.join(&e.path))?,
            Kind::Symlink { target } => symlink(target, root.join(&e.path))?,
            _ => {}
        }
    }

    // Pass 4: permissions. Files and FIFOs in any order, then directories
    // deepest first so restricting a parent never blocks a child chmod.
    for e in entries {
        if let Kind::File { mode, .. } | Kind::Fifo { mode } = &e.kind {
            fs::set_permissions(root.join(&e.path), fs::Permissions::from_mode(*mode))?;
        }
    }
    let mut dirs: Vec<(&Path, u32)> = entries
        .iter()
        .filter_map(|e| match e.kind {
            Kind::Dir { mode } => Some((e.path.as_path(), mode)),
            _ => None,
        })
        .collect();
    dirs.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
    for (path, mode) in dirs {
        fs::set_permissions(root.join(path), fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn write_file(abs: &Path, seed: u64, rel: &Path, content: &Content) -> io::Result<()> {
    let file = fs::File::create(abs)?;
    match content {
        Content::Sparse { len } => {
            // Seek past the hole and write only the final marker byte: the
            // filesystem allocates (at most) one block for a 1 MiB file.
            let mut file = file;
            if *len > 0 {
                file.seek(SeekFrom::Start(len - 1))?;
                file.write_all(b"S")?;
            }
            Ok(())
        }
        other => {
            let mut w = BufWriter::new(file);
            let mut rng = file_rng(seed, rel);
            write_content(&mut w, &mut rng, other)?;
            w.flush()
        }
    }
}

/// Create a FIFO. `std` has no `mkfifo`, and taking a `libc` dependency for
/// one syscall would break the zero-dependency contract, so we shell out to
/// the POSIX-mandated `mkfifo(1)` — present on every Linux and macOS system.
fn mkfifo(path: &Path) -> io::Result<()> {
    let status = Command::new("mkfifo").arg(path).status().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("running mkfifo (needed by the exotic module): {e}"),
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "mkfifo {} failed: {status}",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan, Options};
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    fn workdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fswreck-writer-{}-{tag}", std::process::id()));
        let _ = crate::clean::force_remove(&d);
        d
    }

    #[test]
    fn writes_a_perms_tree_with_exact_modes_regardless_of_umask() {
        let root = workdir("perms");
        let opts = Options {
            modules: vec!["perms".into()],
            ..Options::default()
        };
        let plan = build_plan(&opts).unwrap();
        write_tree(&root, opts.seed, &plan).unwrap();

        let mode = |p: &str| fs::symlink_metadata(root.join(p)).unwrap().mode() & 0o7777;
        assert_eq!(mode("perms/no-read.txt"), 0o000);
        assert_eq!(mode("perms/write-only.txt"), 0o200);
        assert_eq!(mode("perms/no-access-dir"), 0o000);
        assert_eq!(mode("perms/sticky-dir"), 0o1777);
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn sparse_file_has_full_logical_size_but_few_blocks() {
        let root = workdir("sparse");
        let opts = Options {
            modules: vec!["exotic".into()],
            ..Options::default()
        };
        let plan = build_plan(&opts).unwrap();
        write_tree(&root, opts.seed, &plan).unwrap();

        let md = fs::symlink_metadata(root.join("exotic/sparse.bin")).unwrap();
        assert_eq!(md.len(), 1 << 20);
        // st_blocks counts 512-byte units; a dense 1 MiB file needs 2048.
        assert!(
            md.blocks() < 64,
            "sparse file allocated {} blocks — hole not preserved",
            md.blocks()
        );
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn hardlinks_share_an_inode_and_symlink_targets_survive_verbatim() {
        let root = workdir("links");
        let opts = Options {
            modules: vec!["symlinks".into(), "exotic".into()],
            ..Options::default()
        };
        let plan = build_plan(&opts).unwrap();
        write_tree(&root, opts.seed, &plan).unwrap();

        let a = fs::symlink_metadata(root.join("exotic/hardlink-original.txt")).unwrap();
        let b = fs::symlink_metadata(root.join("exotic/hardlink-copy.txt")).unwrap();
        assert_eq!(a.ino(), b.ino());
        assert_eq!(a.nlink(), 2);

        let target = fs::read_link(root.join("symlinks/escape")).unwrap();
        assert_eq!(target, PathBuf::from("../../outside-the-fixture"));
        // The self-loop exists as a link even though it can never resolve.
        assert!(fs::symlink_metadata(root.join("symlinks/self"))
            .unwrap()
            .file_type()
            .is_symlink());
        crate::clean::force_remove(&root).unwrap();
    }
}
