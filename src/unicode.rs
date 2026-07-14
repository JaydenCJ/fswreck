//! `unicode` module: names that break naive string handling.
//!
//! Every entry here is a real-world failure mode: NFC/NFD pairs that look
//! identical but are different files, an RTL override that makes `ls` lie
//! about the extension, zero-width characters, a 255-byte multi-byte name at
//! the ext4/APFS component limit, and a name that is not valid UTF-8 at all
//! (perfectly legal on Linux, fatal to `String`-based path handling).

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use crate::plan::Options;
use crate::spec::{Content, Entry};

pub const NAME: &str = "unicode";
pub const SUMMARY: &str =
    "NFC/NFD pairs, RTL override, zero-width chars, 255-byte names, invalid UTF-8";

fn f(name: &str) -> Entry {
    Entry::file(
        PathBuf::from(NAME).join(name),
        0o644,
        Content::Text { len: 96 },
    )
}

pub fn build(_opts: &Options) -> Vec<Entry> {
    let mut v = vec![Entry::dir(NAME, 0o755)];

    // NFC vs NFD: both render as "café.txt" but are distinct byte sequences.
    // Sync tools that normalize on one side silently merge or duplicate them.
    v.push(f("caf\u{e9}.txt")); // NFC: U+00E9
    v.push(f("cafe\u{301}.txt")); // NFD: 'e' + combining acute

    // Stacked combining marks on one base character.
    v.push(f("a\u{300}\u{301}\u{302}grave.txt"));

    // U+202E RIGHT-TO-LEFT OVERRIDE: displays as "…gpj.txt" reversed, the
    // classic extension-spoofing trick.
    v.push(f("\u{202E}txt.gpj"));

    // Zero-width space and joiner-heavy emoji (11 codepoints, one glyph).
    v.push(f("zero\u{200B}width.txt"));
    v.push(f(
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}-family.txt",
    ));

    // Fullwidth forms, a ligature, and math alphanumerics: all confusable
    // with plain ASCII under casefolding or OCR-ish comparisons.
    v.push(f(
        "\u{FF46}\u{FF55}\u{FF4C}\u{FF4C}\u{FF57}\u{FF49}\u{FF44}\u{FF54}\u{FF48}.txt",
    ));
    v.push(f("\u{FB01}le.txt")); // "ﬁle.txt" with the fi ligature
    v.push(f("\u{1D4EF}\u{1D4EA}\u{1D4F7}\u{1D4EC}\u{1D4FE}.log"));

    // Non-Latin scripts.
    v.push(f("\u{65E5}\u{672C}\u{8A9E}\u{306E}\u{540D}\u{524D}.md"));
    v.push(Entry::file(
        PathBuf::from(NAME).join("\u{3A9}\u{2248}\u{E7}\u{221A}.dat"),
        0o644,
        Content::Bytes { len: 128 },
    ));

    // Exactly 255 bytes — the component limit on ext4 and APFS. 85 × U+3042
    // ("あ", 3 bytes each).
    let long: String = "\u{3042}".repeat(85);
    debug_assert_eq!(long.len(), 255);
    v.push(f(&long));

    // A case-colliding pair: two files on case-sensitive filesystems, one on
    // case-insensitive ones (see the README's macOS note).
    v.push(f("CaseFold.txt"));
    v.push(f("casefold.TXT"));

    // Invalid UTF-8: byte 0xFF can never appear in well-formed UTF-8, yet is
    // a legal Linux filename byte. `Path::to_str()` returns None here.
    let mut raw = b"not-utf8-".to_vec();
    raw.push(0xFF);
    raw.extend_from_slice(b".bin");
    v.push(Entry::file(
        PathBuf::from(NAME).join(PathBuf::from(OsString::from_vec(raw))),
        0o644,
        Content::Bytes { len: 64 },
    ));

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        build(&Options::default())
    }

    #[test]
    fn nfc_and_nfd_names_are_distinct_paths() {
        let v = entries();
        let nfc = PathBuf::from(NAME).join("caf\u{e9}.txt");
        let nfd = PathBuf::from(NAME).join("cafe\u{301}.txt");
        assert!(v.iter().any(|e| e.path == nfc));
        assert!(v.iter().any(|e| e.path == nfd));
        assert_ne!(nfc, nfd);
    }

    #[test]
    fn includes_invalid_utf8_and_a_255_byte_component() {
        use std::os::unix::ffi::OsStrExt;
        let v = entries();
        assert!(
            v.iter()
                .any(|e| e.path.file_name().is_some()
                    && e.path.file_name().unwrap().to_str().is_none()),
            "no invalid-UTF-8 name in the module"
        );
        let max = v
            .iter()
            .filter_map(|e| e.path.file_name())
            .map(|n| n.as_bytes().len())
            .max()
            .unwrap();
        assert_eq!(
            max, 255,
            "longest component must sit at the ext4/APFS limit"
        );
    }

    #[test]
    fn entry_count_is_stable() {
        // The curated set is part of the compatibility contract: adding or
        // removing a fixture is a breaking change to recorded manifests.
        assert_eq!(entries().len(), 16);
    }
}
