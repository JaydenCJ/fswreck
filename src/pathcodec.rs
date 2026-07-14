//! Lossless ASCII encoding for hostile paths.
//!
//! Fixture names contain newlines, control bytes, RTL overrides and invalid
//! UTF-8 — none of which survive a strict JSON string, a terminal, or a
//! diff. Every path in the manifest (and in `fswreck plan` output) is
//! therefore percent-encoded: bytes outside `[A-Za-z0-9._~-]` and `/` become
//! `%XX`. Decoding is exact down to the byte, so a manifest round-trips even
//! for names that are not valid UTF-8.

use std::path::{Path, PathBuf};

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

/// Raw bytes of a path, as the kernel sees them.
pub fn path_bytes(p: &Path) -> &[u8] {
    p.as_os_str().as_bytes()
}

fn is_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/')
}

/// Percent-encode a path into printable ASCII. `/` is kept as the separator.
pub fn encode(p: &Path) -> String {
    encode_bytes(path_bytes(p))
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if is_safe(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode percent-encoded ASCII back into raw bytes. Rejects malformed
/// escapes and any raw byte that the encoder would have escaped, so there is
/// exactly one valid encoding per path.
pub fn decode_bytes(s: &str) -> Result<Vec<u8>, String> {
    let raw = s.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == b'%' {
            let hex = raw
                .get(i + 1..i + 3)
                .ok_or_else(|| format!("truncated %-escape at end of {s:?}"))?;
            let hi = hex_val(hex[0]).ok_or_else(|| bad_escape(s, i))?;
            let lo = hex_val(hex[1]).ok_or_else(|| bad_escape(s, i))?;
            out.push(hi * 16 + lo);
            i += 3;
        } else if is_safe(b) {
            out.push(b);
            i += 1;
        } else {
            return Err(format!("unencoded byte 0x{b:02X} in {s:?}"));
        }
    }
    Ok(out)
}

fn bad_escape(s: &str, i: usize) -> String {
    format!("invalid %-escape at byte {i} of {s:?}")
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decode a manifest path into a relative `PathBuf`, rejecting anything that
/// could escape the fixture root: absolute paths, empty segments, `.` and
/// `..`. A hostile *manifest* must not be able to make `verify` or `clean`
/// stat files outside the tree.
pub fn decode_rel(s: &str) -> Result<PathBuf, String> {
    let bytes = decode_bytes(s)?;
    if bytes.is_empty() {
        return Err("empty path in manifest".into());
    }
    if bytes[0] == b'/' {
        return Err(format!("absolute path in manifest: {s:?}"));
    }
    for seg in bytes.split(|&b| b == b'/') {
        if seg.is_empty() || seg == b"." || seg == b".." {
            return Err(format!("path escapes the fixture root: {s:?}"));
        }
    }
    Ok(bytes_to_pathbuf(bytes))
}

/// Decode a symlink target. Targets are *allowed* to be absolute or contain
/// `..` — escaping links are part of the fixture. They are only ever
/// compared against `readlink`, never traversed by fswreck itself.
pub fn decode_target(s: &str) -> Result<PathBuf, String> {
    Ok(bytes_to_pathbuf(decode_bytes(s)?))
}

fn bytes_to_pathbuf(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_escapes_everything_outside_the_safe_set() {
        assert_eq!(encode(Path::new("a/b/c.txt")), "a/b/c.txt");
        assert_eq!(encode(Path::new("a b")), "a%20b");
        assert_eq!(encode(Path::new("new\nline")), "new%0Aline");
        assert_eq!(encode(Path::new("100%")), "100%25");
        // NFC "café" — the é is two UTF-8 bytes, escaped per byte.
        assert_eq!(encode(Path::new("caf\u{e9}")), "caf%C3%A9");
    }

    #[test]
    fn invalid_utf8_round_trips_exactly() {
        let raw = vec![b'f', b'o', b'o', 0xFF, b'.', b'b', b'i', b'n'];
        let p = bytes_to_pathbuf(raw.clone());
        let enc = encode(&p);
        assert_eq!(enc, "foo%FF.bin");
        assert_eq!(decode_bytes(&enc).unwrap(), raw);
    }

    #[test]
    fn decode_rejects_raw_unsafe_bytes() {
        // A space must arrive as %20; a raw space means the manifest was not
        // produced by (or faithfully to) the encoder.
        assert!(decode_bytes("a b").is_err());
        assert!(decode_bytes("a%2").is_err(), "truncated escape accepted");
        assert!(decode_bytes("a%zz").is_err(), "non-hex escape accepted");
    }

    #[test]
    fn decode_rel_rejects_traversal_but_allows_dot_prefixed_names() {
        assert!(decode_rel("../x").is_err());
        assert!(decode_rel("a/../b").is_err());
        assert!(decode_rel("/etc/hosts").is_err());
        assert!(decode_rel("a//b").is_err());
        assert!(decode_rel("").is_err());
        assert!(decode_rel("a/./b").is_err());
        // `.hidden` and `..almost` are legitimate hostile names; only the
        // exact segments `.` and `..` are traversal.
        assert!(decode_rel(".hidden").is_ok());
        assert!(decode_rel("..almost").is_ok());
        assert!(decode_rel("a/...").is_ok());
    }

    #[test]
    fn decode_target_allows_escaping_links() {
        assert_eq!(
            decode_target("../../outside").unwrap(),
            PathBuf::from("../../outside")
        );
        assert_eq!(
            decode_target("/nonexistent/x").unwrap(),
            PathBuf::from("/nonexistent/x")
        );
    }
}
