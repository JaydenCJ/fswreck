//! End-to-end tests against the compiled `fswreck` binary.
//!
//! Every test works in its own temp directory and tears it down through the
//! binary's own `clean` command (plain `remove_dir_all` chokes on the
//! mode-000 directories the fixture contains — which is the point).

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_fswreck");

fn run<S: AsRef<OsStr>>(args: &[S]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to spawn fswreck")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Unique per-test workdir; the caller generates into `<dir>/tree`.
fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fswreck-it-{}-{tag}", std::process::id()));
    let _ = Command::new(BIN)
        .args(["clean", "--force"])
        .arg(&d)
        .output();
    fs::create_dir_all(&d).unwrap();
    d
}

fn nuke(dir: &Path) {
    let out = Command::new(BIN)
        .args(["clean", "--force"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "cleanup failed: {}", stderr(&out));
}

fn generate(dir: &Path, extra: &[&str]) {
    let mut args: Vec<&OsStr> = vec![OsStr::new("generate"), dir.as_os_str()];
    args.extend(extra.iter().map(OsStr::new));
    let out = run(&args);
    assert!(out.status.success(), "generate failed: {}", stderr(&out));
}

#[test]
fn version_and_help_describe_the_whole_cli() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        stdout(&out).trim(),
        format!("fswreck {}", env!("CARGO_PKG_VERSION"))
    );

    let out = run(&["--help"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("COMMANDS:"));
    for cmd in ["generate", "plan", "verify", "clean", "modules"] {
        assert!(text.contains(cmd), "help missing {cmd}");
    }
}

#[test]
fn unknown_command_is_a_usage_error_on_stderr() {
    let out = run(&["shred"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("unknown command"));
    assert!(stdout(&out).is_empty());
}

#[test]
fn modules_lists_all_six_with_entry_counts() {
    let out = run(&["modules"]);
    assert!(out.status.success());
    let text = stdout(&out);
    for name in ["unicode", "names", "symlinks", "deep", "perms", "exotic"] {
        assert!(text.contains(name), "modules output missing {name}");
    }
    // Header plus six rows.
    assert_eq!(text.lines().count(), 7);
}

#[test]
fn plan_is_deterministic_and_respects_module_subsets() {
    let a = run(&["plan", "--seed", "7"]);
    let b = run(&["plan", "--seed", "7"]);
    assert!(a.status.success());
    assert_eq!(stdout(&a), stdout(&b));
    assert!(stdout(&a).contains("names/-rf"));
    assert!(stdout(&a).contains("symlinks/self -> self"));

    let out = run(&["plan", "--modules", "perms"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("perms/no-access-dir"));
    assert!(!text.contains("unicode/"), "subset leaked other modules");
}

#[test]
fn generate_then_verify_round_trips_clean() {
    let dir = workdir("roundtrip");
    let tree = dir.join("tree");
    generate(&tree, &[]);

    let out = run(&[OsStr::new("verify"), tree.as_os_str()]);
    assert!(
        out.status.success(),
        "verify failed: {}{}",
        stdout(&out),
        stderr(&out)
    );
    let text = stdout(&out);
    assert!(text.contains(": OK"), "{text}");
    assert!(!text.contains("problem:"), "{text}");
    nuke(&dir);
}

#[test]
fn generate_reports_counts_and_writes_the_manifest() {
    let dir = workdir("manifest");
    let tree = dir.join("tree");
    let out = run(&[OsStr::new("generate"), tree.as_os_str()]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("generated"), "{text}");
    assert!(text.contains("seed 42"), "{text}");
    let manifest = fs::read_to_string(tree.join(".fswreck-manifest.json")).unwrap();
    assert!(manifest.contains("\"format\": 1"));
    assert!(manifest.contains("\"seed\": \"42\""));
    nuke(&dir);
}

#[test]
fn generate_refuses_a_non_empty_target_without_force() {
    let dir = workdir("nonempty");
    let tree = dir.join("tree");
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("precious.txt"), b"do not clobber").unwrap();

    let out = run(&[OsStr::new("generate"), tree.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not empty"));
    assert_eq!(
        fs::read(tree.join("precious.txt")).unwrap(),
        b"do not clobber"
    );

    let out = run(&[
        OsStr::new("generate"),
        tree.as_os_str(),
        OsStr::new("--force"),
        OsStr::new("--modules"),
        OsStr::new("exotic"),
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    nuke(&dir);
}

#[test]
fn verify_flags_a_deleted_file() {
    let dir = workdir("deleted");
    let tree = dir.join("tree");
    generate(&tree, &["--modules", "exotic"]);
    fs::remove_file(tree.join("exotic/empty.txt")).unwrap();

    let out = run(&[OsStr::new("verify"), tree.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(
        text.contains("problem: exotic/empty.txt: missing"),
        "{text}"
    );
    // Exactly "1 problem" — a trailing "s" here would be a pluralization bug.
    assert!(text.contains(": 1 problem\n"), "{text}");
    nuke(&dir);
}

#[test]
fn verify_flags_content_tampering() {
    let dir = workdir("content");
    let tree = dir.join("tree");
    generate(&tree, &["--modules", "symlinks"]);
    // Same size, different bytes: only the fingerprint can catch this.
    let payload = tree.join("symlinks/payload.txt");
    let original = fs::read(&payload).unwrap();
    let mut flipped = original.clone();
    flipped[0] ^= 0xFF;
    fs::write(&payload, &flipped).unwrap();

    let out = run(&[OsStr::new("verify"), tree.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("content fingerprint"),
        "{}",
        stdout(&out)
    );
    nuke(&dir);
}

#[test]
fn verify_flags_mode_drift() {
    let dir = workdir("mode");
    let tree = dir.join("tree");
    generate(&tree, &["--modules", "perms"]);
    fs::set_permissions(
        tree.join("perms/read-only.txt"),
        fs::Permissions::from_mode(0o666),
    )
    .unwrap();

    let out = run(&[OsStr::new("verify"), tree.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(text.contains("mode 666, manifest says 444"), "{text}");
    nuke(&dir);
}

#[test]
fn verify_flags_extra_entries() {
    let dir = workdir("extra");
    let tree = dir.join("tree");
    generate(&tree, &["--modules", "names"]);
    fs::write(tree.join("names/impostor"), b"not in the manifest").unwrap();

    let out = run(&[OsStr::new("verify"), tree.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("problem: names/impostor: unexpected entry"),
        "{}",
        stdout(&out)
    );
    nuke(&dir);
}

#[test]
fn verify_supports_an_external_manifest_path() {
    let dir = workdir("external");
    let tree = dir.join("tree");
    let manifest = dir.join("fixture.manifest.json");
    generate(
        &tree,
        &[
            "--modules",
            "unicode",
            "--manifest",
            manifest.to_str().unwrap(),
        ],
    );
    assert!(manifest.exists());
    // No default manifest inside the tree...
    assert!(!tree.join(".fswreck-manifest.json").exists());
    // ...so verify must be pointed at the external one.
    let out = run(&[
        OsStr::new("verify"),
        tree.as_os_str(),
        OsStr::new("--manifest"),
        manifest.as_os_str(),
    ]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    nuke(&dir);
}

#[test]
fn seeds_reproduce_manifests_byte_for_byte_and_only_move_bytes() {
    let dir = workdir("seeds");
    let t1 = dir.join("t1");
    let t2 = dir.join("t2");
    let t3 = dir.join("t3");
    generate(&t1, &["--seed", "123", "--modules", "names"]);
    generate(&t2, &["--seed", "123", "--modules", "names"]);
    generate(&t3, &["--seed", "124", "--modules", "names"]);
    let m1 = fs::read_to_string(t1.join(".fswreck-manifest.json")).unwrap();
    let m2 = fs::read_to_string(t2.join(".fswreck-manifest.json")).unwrap();
    let m3 = fs::read_to_string(t3.join(".fswreck-manifest.json")).unwrap();
    // Same seed: byte-identical manifest. Different seed: fingerprints
    // change but the topology (the path list) does not.
    assert_eq!(m1, m2);
    assert_ne!(m1, m3, "seed had no effect");
    let paths = |m: &str| -> Vec<String> {
        m.lines()
            .filter(|l| l.contains("\"path\""))
            .map(|l| l.split('"').nth(3).unwrap().to_string())
            .collect()
    };
    assert_eq!(paths(&m1), paths(&m3), "seed changed the topology");
    nuke(&dir);
}

#[test]
fn depth_flag_controls_nesting_and_is_recorded() {
    let dir = workdir("depth");
    let tree = dir.join("tree");
    generate(&tree, &["--modules", "deep", "--depth", "3"]);
    let bottom = tree.join("deep/nest/d000/d001/d002/bottom.txt");
    assert!(bottom.exists(), "bottom.txt not at depth 3");
    let manifest = fs::read_to_string(tree.join(".fswreck-manifest.json")).unwrap();
    assert!(manifest.contains("\"depth\": 3"));
    nuke(&dir);
}

#[test]
fn clean_removes_a_full_hostile_tree() {
    let dir = workdir("clean");
    let tree = dir.join("tree");
    generate(&tree, &[]);
    // Sanity: the tree really contains a locked directory.
    let locked = fs::symlink_metadata(tree.join("perms/no-access-dir")).unwrap();
    assert_eq!(locked.permissions().mode() & 0o777, 0);

    let out = run(&[OsStr::new("clean"), tree.as_os_str()]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!tree.exists());
    nuke(&dir);
}

#[test]
fn clean_refuses_directories_it_did_not_generate() {
    let dir = workdir("refuse");
    let victim = dir.join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("data.txt"), b"keep me").unwrap();

    let out = run(&[OsStr::new("clean"), victim.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("refusing"));
    assert!(victim.join("data.txt").exists());
    nuke(&dir);
}
