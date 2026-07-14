//! Replay a manifest against a tree on disk.
//!
//! `verify` answers two questions after your tool ran over the fixture:
//! did every recorded entry survive byte-for-byte (kind, mode, size,
//! content, link target, shared inode), and did anything appear that the
//! manifest does not know about?
//!
//! Nothing is ever *followed*: all checks use `lstat`/`readlink`, so cycles
//! and escaping links are inert. Directories the fixture locked shut
//! (mode 000 & friends) are temporarily relaxed to `700` and restored to
//! whatever mode they actually had — verification never repairs, it reports.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestEntry};
use crate::pathcodec;
use crate::rng::Fnv64;

/// One verification failure, in manifest order.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// Percent-encoded path.
    pub path: String,
    pub detail: String,
}

/// Result of a verification run.
#[derive(Debug)]
pub struct Report {
    /// Number of manifest entries checked.
    pub checked: usize,
    pub problems: Vec<Problem>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Restores relaxed directory modes on drop, children before parents.
struct RelaxGuard {
    relaxed: Vec<(PathBuf, u32)>, // absolute path, observed mode to restore
}

impl RelaxGuard {
    fn new() -> Self {
        Self {
            relaxed: Vec::new(),
        }
    }

    fn relax(&mut self, abs: &Path, observed_mode: u32) {
        if fs::set_permissions(abs, fs::Permissions::from_mode(0o700)).is_ok() {
            self.relaxed.push((abs.to_path_buf(), observed_mode));
        }
    }
}

impl Drop for RelaxGuard {
    fn drop(&mut self) {
        // Deepest first: a parent stays traversable until its children are
        // restored.
        self.relaxed
            .sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
        for (path, mode) in &self.relaxed {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(*mode));
        }
    }
}

/// Verify `root` against `manifest`. `manifest_path` (if inside the root) is
/// exempted from the unexpected-entry scan.
pub fn verify(root: &Path, manifest: &Manifest, manifest_path: &Path) -> Result<Report, String> {
    let mut decoded: Vec<(PathBuf, &ManifestEntry)> = Vec::with_capacity(manifest.entries.len());
    for e in &manifest.entries {
        let rel = pathcodec::decode_rel(&e.path).map_err(|err| format!("bad manifest: {err}"))?;
        decoded.push((rel, e));
    }

    let mut problems = Vec::new();
    let mut guard = RelaxGuard::new();
    // Observed modes of relaxed dirs, so the mode check below sees the real
    // pre-relax value instead of our temporary 700.
    let mut observed: HashMap<PathBuf, u32> = HashMap::new();

    // Relax restrictive directories, shallowest first (manifest order lists
    // parents before children, but sort to be safe against edited files).
    let mut restrictive: Vec<&(PathBuf, &ManifestEntry)> = decoded
        .iter()
        .filter(|(_, e)| e.kind == "dir" && e.mode.is_some_and(|m| m & 0o500 != 0o500))
        .collect();
    restrictive.sort_by_key(|(p, _)| p.components().count());
    for (rel, _) in restrictive {
        let abs = root.join(rel);
        if let Ok(md) = fs::symlink_metadata(&abs) {
            if md.file_type().is_dir() {
                observed.insert(rel.clone(), md.mode() & 0o7777);
                guard.relax(&abs, md.mode() & 0o7777);
            }
        }
    }

    for (rel, entry) in &decoded {
        check_entry(root, rel, entry, &observed, &mut problems);
    }

    // Unexpected-entry scan.
    let expected: HashSet<&Path> = decoded.iter().map(|(p, _)| p.as_path()).collect();
    let manifest_rel = manifest_path.strip_prefix(root).ok().map(Path::to_path_buf);
    let mut extras = Vec::new();
    scan_extras(
        root,
        Path::new(""),
        &expected,
        manifest_rel.as_deref(),
        &mut extras,
    );
    for rel in extras {
        problems.push(Problem {
            path: pathcodec::encode(&rel),
            detail: "unexpected entry (not in manifest)".into(),
        });
    }

    Ok(Report {
        checked: decoded.len(),
        problems,
    })
}

fn push(problems: &mut Vec<Problem>, rel: &Path, detail: String) {
    problems.push(Problem {
        path: pathcodec::encode(rel),
        detail,
    });
}

fn check_entry(
    root: &Path,
    rel: &Path,
    entry: &ManifestEntry,
    observed: &HashMap<PathBuf, u32>,
    problems: &mut Vec<Problem>,
) {
    let abs = root.join(rel);
    let md = match fs::symlink_metadata(&abs) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            push(
                problems,
                rel,
                format!("missing ({} in manifest)", entry.kind),
            );
            return;
        }
        Err(e) => {
            push(problems, rel, format!("cannot stat: {e}"));
            return;
        }
    };

    let ft = md.file_type();
    let actual_kind = if ft.is_symlink() {
        "symlink"
    } else if ft.is_dir() {
        "dir"
    } else if ft.is_fifo() {
        "fifo"
    } else if ft.is_file() {
        "file"
    } else {
        "other"
    };
    // A hardlink is a regular file on disk; its identity check is the inode.
    let expected_kind = if entry.kind == "hardlink" {
        "file"
    } else {
        entry.kind.as_str()
    };
    if actual_kind != expected_kind {
        push(
            problems,
            rel,
            format!("expected {expected_kind}, found {actual_kind}"),
        );
        return;
    }

    // Mode check (symlink modes are meaningless on Linux and not recorded).
    if let Some(want_mode) = entry.mode {
        let got = observed.get(rel).copied().unwrap_or(md.mode() & 0o7777);
        if got != want_mode {
            push(
                problems,
                rel,
                format!("mode {got:o}, manifest says {want_mode:o}"),
            );
        }
    }

    match entry.kind.as_str() {
        "file" => {
            if let Some(want_size) = entry.size {
                if md.len() != want_size {
                    push(
                        problems,
                        rel,
                        format!("size {} bytes, manifest says {want_size}", md.len()),
                    );
                    return; // hash will trivially differ; report the root cause
                }
            }
            if let Some(want_hash) = &entry.fnv1a64 {
                match hash_file(&abs, &md) {
                    Ok(got) => {
                        let got = format!("{got:016x}");
                        if &got != want_hash {
                            push(
                                problems,
                                rel,
                                format!("content fingerprint {got}, manifest says {want_hash}"),
                            );
                        }
                    }
                    Err(e) => push(problems, rel, format!("cannot read: {e}")),
                }
            }
        }
        "symlink" => {
            let want = entry.target.as_ref().unwrap();
            match (fs::read_link(&abs), pathcodec::decode_target(want)) {
                (Ok(got), Ok(want_path)) => {
                    if got != want_path {
                        push(
                            problems,
                            rel,
                            format!("target {}, manifest says {want}", pathcodec::encode(&got)),
                        );
                    }
                }
                (Err(e), _) => push(problems, rel, format!("cannot readlink: {e}")),
                (_, Err(e)) => push(problems, rel, format!("bad manifest target: {e}")),
            }
        }
        "hardlink" => {
            let enc = entry.original.as_ref().unwrap();
            match pathcodec::decode_rel(enc) {
                Ok(orig_rel) => match fs::symlink_metadata(root.join(&orig_rel)) {
                    Ok(orig_md) => {
                        if md.ino() != orig_md.ino() || md.dev() != orig_md.dev() {
                            push(
                                problems,
                                rel,
                                format!("not hardlinked to {enc} (different inode)"),
                            );
                        }
                    }
                    Err(e) => push(problems, rel, format!("original {enc} unreadable: {e}")),
                },
                Err(e) => push(problems, rel, format!("bad manifest original: {e}")),
            }
        }
        _ => {}
    }
}

/// Hash a file's bytes, temporarily granting owner-read if the fixture
/// stripped it (mode 000 / write-only files). The observed mode is restored
/// afterwards.
fn hash_file(abs: &Path, md: &fs::Metadata) -> std::io::Result<u64> {
    let mode = md.mode() & 0o7777;
    let needs_relax = mode & 0o400 == 0;
    if needs_relax {
        fs::set_permissions(abs, fs::Permissions::from_mode(mode | 0o400))?;
    }
    let result = (|| {
        let mut f = fs::File::open(abs)?;
        let mut hasher = Fnv64::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            Fnv64::write(&mut hasher, &buf[..n]);
        }
        Ok(hasher.finish())
    })();
    if needs_relax {
        let _ = fs::set_permissions(abs, fs::Permissions::from_mode(mode));
    }
    result
}

/// Walk the tree (without following symlinks) and collect paths that the
/// manifest does not list.
fn scan_extras(
    root: &Path,
    rel: &Path,
    expected: &HashSet<&Path>,
    manifest_rel: Option<&Path>,
    extras: &mut Vec<PathBuf>,
) {
    let abs = root.join(rel);
    let entries = match fs::read_dir(&abs) {
        Ok(rd) => rd,
        Err(_) => return, // unreadable even after relaxing: reported via mode checks
    };
    let mut children: Vec<_> = entries.filter_map(Result::ok).collect();
    // Deterministic report order.
    children.sort_by_key(|d| d.file_name());
    for child in children {
        let child_rel = rel.join(child.file_name());
        if Some(child_rel.as_path()) == manifest_rel {
            continue;
        }
        if !expected.contains(child_rel.as_path()) {
            extras.push(child_rel.clone());
        }
        if child.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            scan_extras(root, &child_rel, expected, manifest_rel, extras);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::plan::{build_plan, Options};
    use crate::writer::write_tree;

    fn generate(tag: &str, modules: &[&str]) -> (PathBuf, Manifest) {
        let root =
            std::env::temp_dir().join(format!("fswreck-verify-{}-{tag}", std::process::id()));
        let _ = crate::clean::force_remove(&root);
        let opts = Options {
            modules: modules.iter().map(|s| s.to_string()).collect(),
            ..Options::default()
        };
        let plan = build_plan(&opts).unwrap();
        write_tree(&root, opts.seed, &plan).unwrap();
        let m = Manifest::from_plan(&opts, &plan);
        (root, m)
    }

    fn run(root: &Path, m: &Manifest) -> Report {
        verify(root, m, &Manifest::default_path(root)).unwrap()
    }

    #[test]
    fn fresh_tree_verifies_clean() {
        let (root, m) = generate("clean", &["unicode", "symlinks", "perms", "exotic"]);
        let report = run(&root, &m);
        assert!(report.ok(), "problems: {:?}", report.problems);
        assert_eq!(report.checked, m.entries.len());
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn restrictive_modes_are_restored_after_verification() {
        use std::os::unix::fs::PermissionsExt;
        let (root, m) = generate("restore", &["perms"]);
        assert!(run(&root, &m).ok());
        // Verification relaxed these to look inside; they must be back.
        let mode = |p: &str| {
            fs::symlink_metadata(root.join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        assert_eq!(mode("perms/no-access-dir"), 0o000);
        assert_eq!(mode("perms/no-read.txt"), 0o000);
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn detects_content_tampering_behind_a_locked_directory() {
        let (root, m) = generate("tamper", &["perms"]);
        // Reach in (relaxing perms ourselves) and flip one byte.
        let dir = root.join("perms/no-access-dir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(dir.join("hidden.txt"), b"tampered").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();

        let report = run(&root, &m);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(report.problems[0].detail.contains("size"));
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn detects_retargeted_symlinks_broken_hardlinks_and_replaced_fifos() {
        let (root, m) = generate("links", &["symlinks", "exotic"]);
        fs::remove_file(root.join("symlinks/ping")).unwrap();
        std::os::unix::fs::symlink("elsewhere", root.join("symlinks/ping")).unwrap();
        fs::remove_file(root.join("exotic/hardlink-copy.txt")).unwrap();
        fs::write(root.join("exotic/hardlink-copy.txt"), b"now independent").unwrap();
        fs::remove_file(root.join("exotic/fifo.pipe")).unwrap();
        fs::write(root.join("exotic/fifo.pipe"), b"").unwrap();

        let report = run(&root, &m);
        let details: Vec<&str> = report.problems.iter().map(|p| p.detail.as_str()).collect();
        assert!(
            details.iter().any(|d| d.contains("target elsewhere")),
            "{details:?}"
        );
        assert!(
            details.iter().any(|d| d.contains("different inode")),
            "{details:?}"
        );
        assert!(
            details.contains(&"expected fifo, found file"),
            "{details:?}"
        );
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn detects_unexpected_extra_entries_but_not_the_manifest_itself() {
        let (root, m) = generate("extra", &["exotic"]);
        fs::write(Manifest::default_path(&root), m.to_json()).unwrap();
        fs::write(root.join("exotic/stray.txt"), b"who put this here").unwrap();

        let report = run(&root, &m);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert_eq!(report.problems[0].path, "exotic/stray.txt");
        crate::clean::force_remove(&root).unwrap();
    }

    #[test]
    fn rejects_manifests_whose_paths_escape_the_root() {
        let (root, m) = generate("escape", &["exotic"]);
        let mut evil = m.clone();
        evil.entries[0].path = "../../../etc/passwd".into();
        let err = verify(&root, &evil, &Manifest::default_path(&root)).unwrap_err();
        assert!(err.contains("bad manifest"), "{err}");
        crate::clean::force_remove(&root).unwrap();
    }
}
