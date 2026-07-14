# Contributing to fswreck

Thanks for your interest in improving fswreck. Issues, discussions and pull requests are all welcome.

## Getting started

Prerequisites: Rust 1.75 or newer (stable toolchain), on Linux or macOS.

```bash
git clone https://github.com/JaydenCJ/fswreck.git
cd fswreck
cargo build
cargo test
bash scripts/smoke.sh
```

`scripts/smoke.sh` generates a full hostile tree in a temp directory, verifies it, tampers with it in four ways, asserts each tampering is reported, and cleans up through the CLI. It finishes in well under a minute and must print `SMOKE OK`.

## Before you open a pull request

1. `cargo fmt` — formatting is enforced.
2. `cargo clippy --all-targets -- -D warnings` — clippy must be clean.
3. `cargo test` — unit tests and the CLI integration tests must pass.
4. `bash scripts/smoke.sh` — the smoke test must print `SMOKE OK`.
5. Add tests for behavior changes. Planning, encoding and manifest logic lives in pure modules (`plan`, `spec`, `pathcodec`, `json`, `manifest`) that never touch disk; only `writer`, `verify` and `clean` do I/O. Please keep it that way.

## Ground rules

- Keep dependencies at zero. fswreck has no runtime or dev dependencies; adding one needs a very strong justification in the PR description.
- No network calls, ever. fswreck reads and writes local files, nothing else — no telemetry, no update checks.
- The curated fixture set is a compatibility contract: same seed, same tree, byte for byte. Adding, removing or renaming an entry changes recorded manifests, so it lands together with a version bump and a CHANGELOG entry — never silently.
- Safety invariants are non-negotiable: `verify` and `clean` must never follow a symlink, and a manifest must never be able to make them touch paths outside the fixture root.
- Code comments and doc comments are written in English.

## Reporting bugs

Please include your `fswreck --version` output, the exact command line (seed, modules, depth), the filesystem the fixture was generated on (ext4/APFS/tmpfs/…), and the relevant `fswreck verify` lines. Fixture bugs are easiest to fix from a `fswreck plan` line plus what actually landed on disk (`ls -la` / `stat` output).

## Security

If you find a security issue (for example a way to make `verify` or `clean` operate outside the fixture root), please do not open a public issue. Use GitHub's private vulnerability reporting on this repository instead.
