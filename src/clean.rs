//! Safe teardown of a fixture tree.
//!
//! Plain `rm -rf` fails on the trees fswreck makes: a mode-000 directory
//! cannot be listed, so recursive deletion stops dead (as any non-root
//! user). `clean` walks the tree without following symlinks, re-grants
//! owner `rwx` on every directory it descends into, and then deletes.
//!
//! Safety valve: `clean` refuses to delete a directory that does not
//! contain a fswreck manifest unless `--force` is given — a typo'd path
//! should never erase a real directory.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::manifest::DEFAULT_NAME;

/// Remove a fixture tree rooted at `root`.
pub fn clean(root: &Path, force: bool) -> Result<(), String> {
    let md =
        fs::symlink_metadata(root).map_err(|e| format!("cannot stat {}: {e}", root.display()))?;
    if !md.file_type().is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    if !force && !root.join(DEFAULT_NAME).exists() {
        return Err(format!(
            "{} has no {DEFAULT_NAME}; refusing to delete a directory fswreck did not generate (use --force to override)",
            root.display()
        ));
    }
    force_remove(root).map_err(|e| format!("removing {}: {e}", root.display()))
}

/// Recursively delete `path`, repairing directory permissions on the way
/// down. Symlinks are removed as links — never followed.
pub fn force_remove(path: &Path) -> io::Result<()> {
    let md = match fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if md.file_type().is_dir() {
        let mode = md.permissions().mode();
        if mode & 0o700 != 0o700 {
            fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700))?;
        }
        for entry in fs::read_dir(path)? {
            force_remove(&entry?.path())?;
        }
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::plan::{build_plan, Options};
    use crate::writer::write_tree;
    use std::path::PathBuf;

    fn generate(tag: &str, modules: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fswreck-clean-{}-{tag}", std::process::id()));
        let _ = force_remove(&root);
        let opts = Options {
            modules: modules.iter().map(|s| s.to_string()).collect(),
            ..Options::default()
        };
        let plan = build_plan(&opts).unwrap();
        write_tree(&root, opts.seed, &plan).unwrap();
        let m = Manifest::from_plan(&opts, &plan);
        fs::write(Manifest::default_path(&root), m.to_json()).unwrap();
        root
    }

    #[test]
    fn removes_a_tree_with_locked_directories() {
        let root = generate("locked", &["perms", "symlinks"]);
        assert!(root.join("perms/no-access-dir").exists());
        clean(&root, false).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn refuses_a_directory_without_a_manifest() {
        let dir =
            std::env::temp_dir().join(format!("fswreck-clean-{}-precious", std::process::id()));
        let _ = force_remove(&dir);
        fs::create_dir_all(dir.join("data")).unwrap();
        let err = clean(&dir, false).unwrap_err();
        assert!(err.contains("refusing"), "{err}");
        assert!(dir.exists(), "clean deleted an unmanifested directory");
        clean(&dir, true).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn force_remove_deletes_symlinks_without_following_them() {
        let root = generate("links", &["symlinks"]);
        // The `escape` link points above the root; make sure the parent
        // directory the link "targets" is untouched afterwards.
        let sibling = root.parent().unwrap().join("outside-the-fixture");
        fs::write(&sibling, b"survivor").unwrap();
        clean(&root, false).unwrap();
        assert!(!root.exists());
        assert_eq!(fs::read(&sibling).unwrap(), b"survivor");
        fs::remove_file(&sibling).unwrap();
    }
}
