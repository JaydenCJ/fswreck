//! The manifest: a JSON record of everything a generation created.
//!
//! One line per entry, keys in a fixed order, paths percent-encoded — so the
//! manifest is diffable, byte-stable for a given seed, and safe to check
//! into a repo next to the fixture. `verify` replays it against the tree.
//!
//! The seed is stored as a *string* because JSON numbers are IEEE doubles
//! and a u64 seed above 2^53 would silently lose bits.

use std::fmt::Write as _;
use std::path::Path;

use crate::json::{self, Value};
use crate::pathcodec;
use crate::plan::Options;
use crate::spec::{content_hash, Entry, Kind};

/// Default manifest filename, written into the fixture root.
pub const DEFAULT_NAME: &str = ".fswreck-manifest.json";

/// Manifest format version.
pub const FORMAT: u64 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    /// Percent-encoded path relative to the fixture root.
    pub path: String,
    /// `dir` | `file` | `symlink` | `hardlink` | `fifo`.
    pub kind: String,
    /// Permission bits (dir/file/fifo).
    pub mode: Option<u32>,
    /// Logical size in bytes (file only).
    pub size: Option<u64>,
    /// FNV-1a 64 content fingerprint, lowercase hex (file only).
    pub fnv1a64: Option<String>,
    /// Percent-encoded link target (symlink only).
    pub target: Option<String>,
    /// Percent-encoded path of the original (hardlink only).
    pub original: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub seed: u64,
    pub depth: u32,
    pub modules: Vec<String>,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    /// Record a plan, computing content fingerprints as we go.
    pub fn from_plan(opts: &Options, entries: &[Entry]) -> Self {
        let records = entries
            .iter()
            .map(|e| {
                let mut m = ManifestEntry {
                    path: pathcodec::encode(&e.path),
                    kind: e.kind_name().to_string(),
                    mode: None,
                    size: None,
                    fnv1a64: None,
                    target: None,
                    original: None,
                };
                match &e.kind {
                    Kind::Dir { mode } | Kind::Fifo { mode } => m.mode = Some(*mode),
                    Kind::File { mode, content } => {
                        m.mode = Some(*mode);
                        m.size = Some(content.len());
                        m.fnv1a64 = Some(format!(
                            "{:016x}",
                            content_hash(opts.seed, &e.path, content)
                        ));
                    }
                    Kind::Symlink { target } => m.target = Some(pathcodec::encode(target)),
                    Kind::Hardlink { original } => m.original = Some(pathcodec::encode(original)),
                }
                m
            })
            .collect();
        Self {
            seed: opts.seed,
            depth: opts.depth,
            modules: opts.modules.clone(),
            entries: records,
        }
    }

    /// Serialize. Output is deterministic: fixed key order, one entry per line.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(out, "  \"format\": {},", FORMAT);
        out.push_str("  \"tool\": \"fswreck\",\n");
        let _ = writeln!(out, "  \"version\": \"{}\",", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(out, "  \"seed\": \"{}\",", self.seed);
        let _ = writeln!(out, "  \"depth\": {},", self.depth);
        let mods: Vec<String> = self
            .modules
            .iter()
            .map(|m| format!("\"{}\"", json::escape(m)))
            .collect();
        let _ = writeln!(out, "  \"modules\": [{}],", mods.join(", "));
        out.push_str("  \"entries\": [\n");
        for (i, e) in self.entries.iter().enumerate() {
            let mut fields = vec![
                format!("\"path\": \"{}\"", json::escape(&e.path)),
                format!("\"kind\": \"{}\"", json::escape(&e.kind)),
            ];
            if let Some(mode) = e.mode {
                fields.push(format!("\"mode\": \"{mode:o}\""));
            }
            if let Some(size) = e.size {
                fields.push(format!("\"size\": {size}"));
            }
            if let Some(h) = &e.fnv1a64 {
                fields.push(format!("\"fnv1a64\": \"{}\"", json::escape(h)));
            }
            if let Some(t) = &e.target {
                fields.push(format!("\"target\": \"{}\"", json::escape(t)));
            }
            if let Some(o) = &e.original {
                fields.push(format!("\"original\": \"{}\"", json::escape(o)));
            }
            let comma = if i + 1 == self.entries.len() { "" } else { "," };
            let _ = writeln!(out, "    {{{}}}{comma}", fields.join(", "));
        }
        out.push_str("  ]\n}\n");
        out
    }

    /// Parse and validate a manifest document.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = json::parse(text).map_err(|e| format!("manifest is not valid JSON: {e}"))?;
        let format = doc
            .get("format")
            .and_then(Value::as_u64)
            .ok_or("manifest missing \"format\"")?;
        if format != FORMAT {
            return Err(format!(
                "unsupported manifest format {format} (this fswreck reads format {FORMAT})"
            ));
        }
        let seed = doc
            .get("seed")
            .and_then(Value::as_str)
            .ok_or("manifest missing \"seed\" (a decimal string)")?
            .parse::<u64>()
            .map_err(|e| format!("invalid seed: {e}"))?;
        let depth = doc
            .get("depth")
            .and_then(Value::as_u64)
            .ok_or("manifest missing \"depth\"")? as u32;
        let modules = doc
            .get("modules")
            .and_then(Value::as_arr)
            .ok_or("manifest missing \"modules\"")?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("\"modules\" must be an array of strings")?;
        let raw_entries = doc
            .get("entries")
            .and_then(Value::as_arr)
            .ok_or("manifest missing \"entries\"")?;
        let mut entries = Vec::with_capacity(raw_entries.len());
        for (i, v) in raw_entries.iter().enumerate() {
            entries.push(parse_entry(v).map_err(|e| format!("entry {i}: {e}"))?);
        }
        Ok(Self {
            seed,
            depth,
            modules,
            entries,
        })
    }

    /// Default manifest location for a fixture root.
    pub fn default_path(root: &Path) -> std::path::PathBuf {
        root.join(DEFAULT_NAME)
    }
}

fn parse_entry(v: &Value) -> Result<ManifestEntry, String> {
    let path = v
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing \"path\"")?
        .to_string();
    let kind = v
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("missing \"kind\"")?
        .to_string();
    if !matches!(
        kind.as_str(),
        "dir" | "file" | "symlink" | "hardlink" | "fifo"
    ) {
        return Err(format!("unknown kind {kind:?}"));
    }
    let mode = match v.get("mode") {
        Some(m) => Some(
            u32::from_str_radix(m.as_str().ok_or("\"mode\" must be an octal string")?, 8)
                .map_err(|e| format!("invalid mode: {e}"))?,
        ),
        None => None,
    };
    let size = match v.get("size") {
        Some(s) => Some(
            s.as_u64()
                .ok_or("\"size\" must be a non-negative integer")?,
        ),
        None => None,
    };
    let take_str = |key: &str| -> Result<Option<String>, String> {
        match v.get(key) {
            Some(s) => Ok(Some(
                s.as_str()
                    .ok_or(format!("\"{key}\" must be a string"))?
                    .to_string(),
            )),
            None => Ok(None),
        }
    };
    // Required-field checks per kind keep tampered manifests from producing
    // confusing verify output later.
    let entry = ManifestEntry {
        path,
        kind: kind.clone(),
        mode,
        size,
        fnv1a64: take_str("fnv1a64")?,
        target: take_str("target")?,
        original: take_str("original")?,
    };
    match kind.as_str() {
        "dir" | "fifo" if entry.mode.is_none() => Err(format!("{kind} entry missing \"mode\"")),
        "file" if entry.mode.is_none() || entry.size.is_none() || entry.fnv1a64.is_none() => {
            Err("file entry missing \"mode\", \"size\" or \"fnv1a64\"".into())
        }
        "symlink" if entry.target.is_none() => Err("symlink entry missing \"target\"".into()),
        "hardlink" if entry.original.is_none() => Err("hardlink entry missing \"original\"".into()),
        _ => Ok(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{build_plan, Options};

    fn sample() -> Manifest {
        let opts = Options::default();
        let plan = build_plan(&opts).unwrap();
        Manifest::from_plan(&opts, &plan)
    }

    #[test]
    fn round_trips_through_json_losslessly_and_byte_stably() {
        let m = sample();
        let parsed = Manifest::parse(&m.to_json()).unwrap();
        assert_eq!(parsed, m);
        assert_eq!(sample().to_json(), m.to_json());
        // Every file entry carries a well-formed 16-hex-digit fingerprint.
        for e in &m.entries {
            if e.kind == "file" {
                let h = e.fnv1a64.as_ref().unwrap();
                assert_eq!(h.len(), 16, "{}: {h}", e.path);
                assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
    }

    #[test]
    fn modes_round_trip_as_octal_including_sticky_and_zero() {
        let m = sample();
        let parsed = Manifest::parse(&m.to_json()).unwrap();
        let sticky = parsed
            .entries
            .iter()
            .find(|e| e.path == "perms/sticky-dir")
            .unwrap();
        assert_eq!(sticky.mode, Some(0o1777));
        let zero = parsed
            .entries
            .iter()
            .find(|e| e.path == "perms/no-read.txt")
            .unwrap();
        assert_eq!(zero.mode, Some(0));
    }

    #[test]
    fn seed_survives_the_full_u64_range() {
        // 2^63 + 7 is not representable as an f64 integer; the string form is.
        let opts = Options {
            seed: (1u64 << 63) + 7,
            modules: vec!["perms".into()],
            ..Options::default()
        };
        let m = Manifest::from_plan(&opts, &build_plan(&opts).unwrap());
        assert_eq!(
            Manifest::parse(&m.to_json()).unwrap().seed,
            (1u64 << 63) + 7
        );
    }

    #[test]
    fn rejects_unsupported_formats_and_unknown_kinds() {
        let doc = sample()
            .to_json()
            .replace("\"format\": 1", "\"format\": 99");
        let err = Manifest::parse(&doc).unwrap_err();
        assert!(err.contains("format 99"), "{err}");
        let doc = sample()
            .to_json()
            .replacen("\"kind\": \"dir\"", "\"kind\": \"socket\"", 1);
        assert!(Manifest::parse(&doc).unwrap_err().contains("unknown kind"));
        // Kind-specific required fields are enforced too.
        let doc = r#"{
          "format": 1, "tool": "fswreck", "version": "0.1.0",
          "seed": "1", "depth": 1, "modules": [],
          "entries": [{"path": "x", "kind": "symlink"}]
        }"#;
        let err = Manifest::parse(doc).unwrap_err();
        assert!(err.contains("target"), "{err}");
    }
}
