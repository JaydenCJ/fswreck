//! `deep` module: nesting depth and path length.
//!
//! Two branches: `nest/` goes `--depth` directories down (recursion-happy
//! walkers and fixed-size path buffers die here first), and `longpath/`
//! builds a relative path just over 2 KiB out of 200-character components —
//! comfortably legal on Linux (PATH_MAX 4096) yet far beyond Windows'
//! classic 260-character limit and many tools' assumptions.

use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "deep";
pub const SUMMARY: &str =
    "--depth nested directories plus a >2KiB relative path of 200-char components";

/// Number of 200-character components in the `longpath` branch. Eleven of
/// them put the relative path at 2,232 bytes.
pub const LONGPATH_COMPONENTS: u32 = 11;

pub fn build(opts: &Options) -> Vec<Entry> {
    let mut v = vec![Entry::dir(NAME, 0o755)];

    // deep/nest/d000/d001/.../bottom.txt
    v.push(Entry::dir(PathBuf::from(NAME).join("nest"), 0o755));
    let mut p = PathBuf::from(NAME).join("nest");
    for i in 0..opts.depth {
        p = p.join(format!("d{i:03}"));
        v.push(Entry::dir(p.clone(), 0o755));
    }
    v.push(Entry::file(
        p.join("bottom.txt"),
        0o644,
        Content::Text { len: 64 },
    ));

    // deep/longpath/xxx…/xxx…/end.txt — each component 200 chars.
    v.push(Entry::dir(PathBuf::from(NAME).join("longpath"), 0o755));
    let mut p = PathBuf::from(NAME).join("longpath");
    for i in 0..LONGPATH_COMPONENTS {
        let component = format!("{}{i:03}", "x".repeat(197));
        p = p.join(component);
        v.push(Entry::dir(p.clone(), 0o755));
    }
    v.push(Entry::file(
        p.join("end.txt"),
        0o644,
        Content::Text { len: 64 },
    ));

    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn bottom_file_sits_at_the_requested_depth() {
        let opts = Options {
            depth: 7,
            ..Options::default()
        };
        let v = build(&opts);
        let bottom = v
            .iter()
            .find(|e| e.path.file_name().is_some_and(|n| n == "bottom.txt"))
            .unwrap();
        // deep + nest + 7 dirs + file name = 10 components.
        assert_eq!(bottom.path.components().count(), 10);
    }

    #[test]
    fn depth_option_scales_the_entry_count() {
        let mut opts = Options {
            depth: 1,
            ..Options::default()
        };
        let small = build(&opts).len();
        opts.depth = 5;
        let large = build(&opts).len();
        assert_eq!(large - small, 4, "one extra dir per depth step");
    }

    #[test]
    fn longpath_uses_200_byte_components_and_exceeds_two_kib_total() {
        let v = build(&Options::default());
        let end = v
            .iter()
            .find(|e| e.path.file_name().is_some_and(|n| n == "end.txt"))
            .unwrap();
        let long_components: Vec<_> = end
            .path
            .components()
            .filter(|c| c.as_os_str().as_bytes().len() == 200)
            .collect();
        assert_eq!(long_components.len(), LONGPATH_COMPONENTS as usize);
        let len = end.path.as_os_str().as_bytes().len();
        assert!(len > 2048, "relative path only {len} bytes");
        assert!(len < 3000, "leave headroom for the caller's root prefix");
    }
}
