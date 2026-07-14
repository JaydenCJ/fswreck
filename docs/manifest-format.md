# The fswreck manifest format

Every `fswreck generate` writes a manifest — by default `.fswreck-manifest.json`
in the fixture root — recording exactly what was created. `fswreck verify`
replays it against the tree; nothing else is consulted.

## Design constraints

1. **Byte-stable.** The same seed and options must serialize to the same
   bytes, so a manifest can be committed next to a fixture and diffed.
2. **Hostile-name-proof.** Fixture names contain newlines, control bytes and
   invalid UTF-8. JSON strings cannot carry arbitrary bytes, so every path is
   **percent-encoded**: bytes outside `[A-Za-z0-9._~-]` and `/` become `%XX`
   (uppercase hex). Decoding is exact down to the byte.
3. **Precision-proof.** JSON numbers are IEEE doubles. The u64 `seed` is
   stored as a *decimal string* so seeds above 2^53 survive round-trips.
   Sizes stay numeric (the largest fixture is 1 MiB).

## Document layout

```json
{
  "format": 1,
  "tool": "fswreck",
  "version": "0.1.0",
  "seed": "42",
  "depth": 32,
  "modules": ["unicode", "names", "symlinks", "deep", "perms", "exotic"],
  "entries": [
    {"path": "unicode", "kind": "dir", "mode": "755"},
    {"path": "unicode/caf%C3%A9.txt", "kind": "file", "mode": "644", "size": 96, "fnv1a64": "40a00aa8247fdfbb"},
    {"path": "symlinks/self", "kind": "symlink", "target": "self"},
    {"path": "exotic/hardlink-copy.txt", "kind": "hardlink", "original": "exotic/hardlink-original.txt"},
    {"path": "exotic/fifo.pipe", "kind": "fifo", "mode": "644"}
  ]
}
```

Entries appear one per line, keys in a fixed order, in plan order (parents
before children, hardlink originals before their links).

## Entry fields

| Field | Present on | Meaning |
|---|---|---|
| `path` | all | Percent-encoded path, relative to the fixture root |
| `kind` | all | `dir`, `file`, `symlink`, `hardlink`, or `fifo` |
| `mode` | dir, file, fifo | Permission bits as an **octal string** (`"1777"`, `"0"`) |
| `size` | file | Logical size in bytes (sparse files report the full size) |
| `fnv1a64` | file | FNV-1a 64-bit fingerprint of the content, 16 hex digits |
| `target` | symlink | Percent-encoded link target, compared against `readlink` |
| `original` | hardlink | Path whose inode the link must share |

## Verification semantics

- Every check uses `lstat`/`readlink` — links are never followed, so cycles
  and escaping targets are inert.
- Manifest `path` values are rejected if absolute or containing `.` / `..`
  segments: a hostile manifest cannot make `verify` touch files outside the
  root. Symlink `target` values are exempt — escaping targets are fixtures.
- Directories whose recorded mode lacks owner `r-x` are temporarily relaxed
  to `700` so their contents can be checked, then restored to whatever mode
  they actually had. Verification reports; it never repairs.
- After all recorded entries are checked, the tree is walked and anything
  not in the manifest is reported as `unexpected entry` (the manifest file
  itself is exempt).

## Compatibility

`format` is bumped on any breaking change to this layout; fswreck refuses
manifests with a format it does not understand. The curated fixture set is
part of the same contract: adding, removing or renaming an entry only
happens together with a version bump, because it changes recorded manifests.
