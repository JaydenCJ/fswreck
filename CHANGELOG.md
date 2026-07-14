# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-13

### Added

- Six curated wreck modules (317 entries at default settings): `unicode` (NFC/NFD confusable pairs, RTL override, zero-width characters, fullwidth/ligature/math-alphanumeric confusables, a 255-byte multi-byte name, an invalid-UTF-8 name), `names` (flag-like names such as `-rf` and `--help`, embedded newlines/tabs/carriage returns, shell and glob metacharacters, Windows reserved device names, a 255-byte ASCII name), `symlinks` (self-loop, A↔B cycle, dangling/absolute/escaping targets, directory loops, a 50-link ELOOP chain), `deep` (`--depth` nested directories plus a 2.2 KiB relative path of 200-character components), `perms` (mode-000 files and directories, write-only/exec-only files, no-exec and no-read directories, sticky bit), and `exotic` (FIFO, hardlink pair, 1 MiB sparse file, empty file and directory, 128-file wide directory).
- `fswreck generate`: materializes the tree with exact modes regardless of umask (restrictive directory modes applied deepest-first, after population) and refuses non-empty targets without `--force`.
- Deterministic content model: file bytes derive from `seed XOR fnv1a64(path)`, so the same seed reproduces a byte-identical tree and manifest, and changing the seed changes bytes but never topology.
- JSON manifest (format 1) with percent-encoded paths that round-trip invalid UTF-8, octal-string modes, string-encoded u64 seed, and an FNV-1a 64 fingerprint per file — see `docs/manifest-format.md`.
- `fswreck verify`: replays the manifest with `lstat`/`readlink` only (never follows links), checks kind/mode/size/fingerprint/link-target/shared-inode, temporarily relaxes and restores unreadable directories, reports entries the manifest does not list, and rejects manifests whose paths would escape the fixture root.
- `fswreck clean`: permission-repairing recursive deletion that survives mode-000 directories and refuses to delete directories without a fswreck manifest unless `--force` is given.
- `fswreck plan` (disk-free preview) and `fswreck modules` (catalog with entry counts).
- Broken-pipe-safe stdout: `fswreck plan | head` exits cleanly like a Unix filter.
- Zero dependencies — runtime and dev; std-only SplitMix64, FNV-1a and JSON parser.
- Test suite: 74 unit tests, 16 CLI integration tests, and `scripts/smoke.sh`.

[0.1.0]: https://github.com/JaydenCJ/fswreck/releases/tag/v0.1.0
