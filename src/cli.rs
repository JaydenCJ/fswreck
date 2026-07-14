//! Argument parsing and command dispatch — hand-rolled, std only.
//!
//! Exit codes: `0` success, `1` verification problems or a runtime failure,
//! `2` usage errors.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use crate::catalog;
use crate::clean;
use crate::manifest::Manifest;
use crate::pathcodec;
use crate::plan::{build_plan, Options, DEFAULT_DEPTH, DEFAULT_SEED};
use crate::spec::{Entry, Kind};
use crate::verify;
use crate::writer;

const USAGE: &str = "\
fswreck — deterministically generates adversarial file trees.

USAGE:
    fswreck <COMMAND> [OPTIONS]

COMMANDS:
    generate <DIR>    Materialize a hostile fixture tree at DIR
    plan              Print the entries a generation would create (no disk I/O)
    verify <DIR>      Check a fixture tree against its manifest
    clean <DIR>       Repair permissions and delete a fixture tree
    modules           List the built-in wreck modules

OPTIONS:
    --seed <N>         Content seed, u64 (default: 42)
    --modules <LIST>   Comma-separated module subset (default: all)
    --depth <N>        Nesting depth for the deep module, 1-512 (default: 32)
    --manifest <PATH>  Manifest file (default: <DIR>/.fswreck-manifest.json)
    --force            generate: allow a non-empty DIR; clean: skip the manifest check
    -h, --help         Show this help
    -V, --version      Show the version

Paths in all output are percent-encoded (see docs/manifest-format.md).";

/// Parsed command line.
#[derive(Debug, PartialEq)]
pub struct Args {
    pub command: Command,
    pub seed: u64,
    pub depth: u32,
    pub modules: Option<Vec<String>>,
    pub manifest: Option<PathBuf>,
    pub force: bool,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Generate(PathBuf),
    Plan,
    Verify(PathBuf),
    Clean(PathBuf),
    Modules,
    Help,
    Version,
}

/// Parse `args` (without the program name).
pub fn parse_args(args: &[OsString]) -> Result<Args, String> {
    let mut command: Option<Command> = None;
    let mut dir_expected = false;
    let mut parsed = Args {
        command: Command::Help,
        seed: DEFAULT_SEED,
        depth: DEFAULT_DEPTH,
        modules: None,
        manifest: None,
        force: false,
    };

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let text = arg.to_str();
        match text {
            Some("-h") | Some("--help") => {
                return Ok(Args {
                    command: Command::Help,
                    ..parsed
                })
            }
            Some("-V") | Some("--version") => {
                return Ok(Args {
                    command: Command::Version,
                    ..parsed
                })
            }
            Some("--force") => parsed.force = true,
            Some("--seed") => {
                parsed.seed = flag_value(&mut it, "--seed")?
                    .parse::<u64>()
                    .map_err(|e| format!("--seed: {e}"))?;
            }
            Some("--depth") => {
                parsed.depth = flag_value(&mut it, "--depth")?
                    .parse::<u32>()
                    .map_err(|e| format!("--depth: {e}"))?;
            }
            Some("--modules") => {
                let list = flag_value(&mut it, "--modules")?;
                parsed.modules = Some(
                    list.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
            }
            Some("--manifest") => {
                let v = it
                    .next()
                    .ok_or_else(|| "--manifest requires a value".to_string())?;
                parsed.manifest = Some(PathBuf::from(v));
            }
            Some(word) if command.is_none() && !word.starts_with('-') => match word {
                "generate" => {
                    command = Some(Command::Generate(PathBuf::new()));
                    dir_expected = true;
                }
                "plan" => command = Some(Command::Plan),
                "verify" => {
                    command = Some(Command::Verify(PathBuf::new()));
                    dir_expected = true;
                }
                "clean" => {
                    command = Some(Command::Clean(PathBuf::new()));
                    dir_expected = true;
                }
                "modules" => command = Some(Command::Modules),
                other => return Err(format!("unknown command {other:?}")),
            },
            _ if dir_expected => {
                let dir = PathBuf::from(arg);
                command = match command {
                    Some(Command::Generate(_)) => Some(Command::Generate(dir)),
                    Some(Command::Verify(_)) => Some(Command::Verify(dir)),
                    Some(Command::Clean(_)) => Some(Command::Clean(dir)),
                    other => other,
                };
                dir_expected = false;
            }
            _ => return Err(format!("unexpected argument {arg:?}")),
        }
    }

    parsed.command = command.ok_or("no command given (try --help)")?;
    if let Command::Generate(d) | Command::Verify(d) | Command::Clean(d) = &parsed.command {
        if d.as_os_str().is_empty() {
            return Err("this command needs a <DIR> argument".into());
        }
    }
    Ok(parsed)
}

fn flag_value<'a, I: Iterator<Item = &'a OsString>>(
    it: &mut I,
    flag: &str,
) -> Result<&'a str, String> {
    it.next()
        .and_then(|v| v.to_str())
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// Write one line to stdout. When the read end of a pipe closes early
/// (`fswreck plan | head`), exit 0 quietly like a well-behaved Unix filter
/// instead of panicking on `Broken pipe`.
fn outln(args: std::fmt::Arguments) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_fmt(args).and_then(|()| out.write_all(b"\n")) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("fswreck: writing to stdout: {e}");
        std::process::exit(1);
    }
}

/// Entry point used by `main`. Returns the process exit code.
pub fn run(args: &[OsString]) -> i32 {
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("fswreck: {e}");
            eprintln!("Run `fswreck --help` for usage.");
            return 2;
        }
    };
    match dispatch(parsed) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fswreck: {e}");
            1
        }
    }
}

fn options_from(args: &Args) -> Options {
    Options {
        seed: args.seed,
        depth: args.depth,
        modules: args.modules.clone().unwrap_or_else(catalog::all_names),
    }
}

fn dispatch(args: Args) -> Result<i32, String> {
    match &args.command {
        Command::Help => {
            outln(format_args!("{USAGE}"));
            Ok(0)
        }
        Command::Version => {
            outln(format_args!("fswreck {}", env!("CARGO_PKG_VERSION")));
            Ok(0)
        }
        Command::Modules => {
            let opts = Options::default();
            outln(format_args!(
                "{:<10} {:>7}  WHAT IT WRECKS",
                "MODULE", "ENTRIES"
            ));
            for info in catalog::all() {
                let count = (info.build)(&opts).len();
                outln(format_args!(
                    "{:<10} {:>7}  {}",
                    info.name, count, info.summary
                ));
            }
            Ok(0)
        }
        Command::Plan => {
            let opts = options_from(&args);
            let entries = build_plan(&opts)?;
            outln(format_args!(
                "{:<9} {:>5} {:>8}  PATH",
                "KIND", "MODE", "SIZE"
            ));
            for e in &entries {
                outln(format_args!("{}", plan_line(e)));
            }
            outln(format_args!(
                "{} entries (seed {}, depth {}, modules: {})",
                entries.len(),
                opts.seed,
                opts.depth,
                opts.modules.join(", ")
            ));
            Ok(0)
        }
        Command::Generate(dir) => {
            let opts = options_from(&args);
            let entries = build_plan(&opts)?;
            if let Ok(read) = fs::read_dir(dir) {
                if read.count() > 0 && !args.force {
                    return Err(format!(
                        "{} is not empty (use --force to generate anyway)",
                        dir.display()
                    ));
                }
            }
            writer::write_tree(dir, opts.seed, &entries)
                .map_err(|e| format!("generating under {}: {e}", dir.display()))?;
            let manifest = Manifest::from_plan(&opts, &entries);
            let manifest_path = args
                .manifest
                .clone()
                .unwrap_or_else(|| Manifest::default_path(dir));
            fs::write(&manifest_path, manifest.to_json())
                .map_err(|e| format!("writing {}: {e}", manifest_path.display()))?;
            outln(format_args!(
                "generated {} entries under {} (seed {}, depth {}, modules: {})",
                entries.len(),
                dir.display(),
                opts.seed,
                opts.depth,
                opts.modules.join(", ")
            ));
            outln(format_args!("manifest: {}", manifest_path.display()));
            Ok(0)
        }
        Command::Verify(dir) => {
            let manifest_path = args
                .manifest
                .clone()
                .unwrap_or_else(|| Manifest::default_path(dir));
            let text = fs::read_to_string(&manifest_path)
                .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
            let manifest = Manifest::parse(&text)?;
            let report = verify::verify(dir, &manifest, &manifest_path)?;
            for p in &report.problems {
                outln(format_args!("problem: {}: {}", p.path, p.detail));
            }
            if report.ok() {
                outln(format_args!(
                    "verified {} {} under {}: OK",
                    report.checked,
                    plural(report.checked, "entry", "entries"),
                    dir.display()
                ));
                Ok(0)
            } else {
                let n = report.problems.len();
                outln(format_args!(
                    "verified {} {} under {}: {} {}",
                    report.checked,
                    plural(report.checked, "entry", "entries"),
                    dir.display(),
                    n,
                    plural(n, "problem", "problems")
                ));
                Ok(1)
            }
        }
        Command::Clean(dir) => {
            clean::clean(dir, args.force)?;
            outln(format_args!("removed {}", dir.display()));
            Ok(0)
        }
    }
}

/// Pick the right noun form: `1 problem`, `3 problems`.
fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        one
    } else {
        many
    }
}

fn plan_line(e: &Entry) -> String {
    let (mode, size) = match &e.kind {
        Kind::Dir { mode } | Kind::Fifo { mode } => (format!("{mode:o}"), "-".to_string()),
        Kind::File { mode, content } => (format!("{mode:o}"), content.len().to_string()),
        Kind::Symlink { .. } | Kind::Hardlink { .. } => ("-".to_string(), "-".to_string()),
    };
    let path = pathcodec::encode(&e.path);
    let suffix = match &e.kind {
        Kind::Symlink { target } => format!(" -> {}", pathcodec::encode(target)),
        Kind::Hardlink { original } => format!(" => {}", pathcodec::encode(original)),
        _ => String::new(),
    };
    format!(
        "{:<9} {:>5} {:>8}  {}{}",
        e.kind_name(),
        mode,
        size,
        path,
        suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_generate_with_all_flags() {
        let a = parse_args(&os(&[
            "generate",
            "/tmp/x",
            "--seed",
            "7",
            "--depth",
            "3",
            "--modules",
            "unicode,perms",
            "--force",
        ]))
        .unwrap();
        assert_eq!(a.command, Command::Generate(PathBuf::from("/tmp/x")));
        assert_eq!(a.seed, 7);
        assert_eq!(a.depth, 3);
        assert_eq!(a.modules, Some(vec!["unicode".into(), "perms".into()]));
        assert!(a.force);
        // Flags may also precede the command word.
        let a = parse_args(&os(&["--seed", "9", "plan"])).unwrap();
        assert_eq!(a.command, Command::Plan);
        assert_eq!(a.seed, 9);
    }

    #[test]
    fn missing_dir_is_a_usage_error() {
        let err = parse_args(&os(&["generate"])).unwrap_err();
        assert!(err.contains("<DIR>"), "{err}");
    }

    #[test]
    fn bad_numbers_unknown_commands_and_stray_args_are_rejected() {
        assert!(parse_args(&os(&["plan", "--seed", "banana"])).is_err());
        assert!(parse_args(&os(&["plan", "--depth", "-1"])).is_err());
        assert!(parse_args(&os(&["plan", "--seed"])).is_err());
        assert!(parse_args(&os(&["explode"])).is_err());
        assert!(parse_args(&os(&["plan", "extra-arg"])).is_err());
        assert!(parse_args(&os(&[])).is_err());
    }

    #[test]
    fn help_and_version_win_wherever_they_appear() {
        assert_eq!(parse_args(&os(&["--help"])).unwrap().command, Command::Help);
        assert_eq!(
            parse_args(&os(&["generate", "/x", "--version"]))
                .unwrap()
                .command,
            Command::Version
        );
    }

    #[test]
    fn dir_arguments_accept_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let dir = OsString::from_vec(vec![b'/', b't', 0xFF]);
        let a = parse_args(&[OsString::from("clean"), dir.clone()]).unwrap();
        assert_eq!(a.command, Command::Clean(PathBuf::from(dir)));
    }

    #[test]
    fn plan_lines_are_fixed_width_and_ascii() {
        let opts = Options::default();
        for e in build_plan(&opts).unwrap() {
            let line = plan_line(&e);
            assert!(line.is_ascii(), "non-ascii plan line: {line}");
        }
    }
}
