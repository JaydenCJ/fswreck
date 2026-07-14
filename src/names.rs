//! `names` module: ASCII names that break shells, globs, and option parsers.
//!
//! No unicode here — these are the boring-looking names that still take down
//! scripts every week: a file literally called `-rf`, names with embedded
//! newlines (the classic `find | xargs` killer), glob metacharacters,
//! Windows reserved device names, and a name that *looks* percent-encoded.

use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "names";
pub const SUMMARY: &str =
    "flag-like names (-rf), embedded newlines/tabs, glob and shell metacharacters";

/// The curated list. Each name is one specific bug class.
const HOSTILE_NAMES: &[&str] = &[
    // Option-injection: passed unquoted to a tool, these become flags.
    "-",
    "--",
    "-rf",
    "-n",
    "--help",
    // Whitespace abuse.
    " leading-space",
    "trailing-space ",
    "double  space",
    "new\nline",
    "tab\tseparated",
    "carriage\rreturn",
    // Shell metacharacters: every one must survive quoting.
    "back\\slash",
    "quote\"double",
    "quote'single",
    "`backtick`",
    "dollar$HOME",
    "semi;colon",
    "pipe|pipe",
    "amp&ersand",
    "redirect>out",
    // Glob metacharacters: `rm fixture/*star*` should not fan out.
    "glob*star",
    "quest?ion",
    "brack[et]s",
    "{brace,set}",
    "~tilde",
    "#hash",
    "!bang",
    // Windows reserved device names — legal on Unix, undeletable on NTFS.
    "CON",
    "NUL",
    "COM1",
    // Trailing dot / dot-heavy names (Windows strips trailing dots).
    "ends-with-dot.",
    "...",
    "..almost-parent",
    // Looks URL-encoded; decode-happy layers turn it into a traversal.
    "%2e%2e",
    // Colons break scp-style remote syntax and NTFS alternate streams.
    "name:with:colons",
];

pub fn build(_opts: &Options) -> Vec<Entry> {
    let mut v = vec![Entry::dir(NAME, 0o755)];
    for name in HOSTILE_NAMES {
        v.push(Entry::file(
            PathBuf::from(NAME).join(name),
            0o644,
            Content::Text { len: 64 },
        ));
    }
    // 255 × 'a': the longest legal ASCII component on ext4/APFS.
    let long = "a".repeat(255);
    v.push(Entry::file(
        PathBuf::from(NAME).join(long),
        0o644,
        Content::Text { len: 64 },
    ));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_the_classic_rm_rf_trap() {
        let v = build(&Options::default());
        assert!(v.iter().any(|e| e.path == PathBuf::from(NAME).join("-rf")));
        assert!(v
            .iter()
            .any(|e| e.path == PathBuf::from(NAME).join("--help")));
    }

    #[test]
    fn includes_an_embedded_newline() {
        let v = build(&Options::default());
        assert!(v
            .iter()
            .any(|e| e.path == PathBuf::from(NAME).join("new\nline")));
    }

    #[test]
    fn no_name_is_an_actual_traversal_segment() {
        // "..almost-parent" and "..." are fine; a literal ".." would make the
        // plan write outside its root.
        for e in build(&Options::default()) {
            let name = e.path.file_name().unwrap();
            assert_ne!(name, "..");
            assert_ne!(name, ".");
        }
    }

    #[test]
    fn entry_count_is_stable() {
        // 1 dir + 35 curated names + 1 long name.
        assert_eq!(build(&Options::default()).len(), 37);
    }
}
