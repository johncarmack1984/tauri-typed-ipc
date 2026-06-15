//! Command-line front end.
//!
//! - `ttipc-migrate <FILE.rs>` reads one TauRPC source file and writes the
//!   migrated ttipc source to stdout (a preview; the original is untouched).
//! - `ttipc-migrate --write <FILE.rs>...` migrates a whole project in place:
//!   it builds one event registry across every file (so an emit site resolves
//!   even when its trait is in another file), edits each file surgically (only
//!   the changed spans, comments preserved), and lands the result as a single
//!   commit on a fresh branch -- so the diff is reviewable and easy to drop.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Branch the in-place migration commit lands on.
const BRANCH: &str = "ttipc-migration";
const COMMIT_MESSAGE: &str = "migrate from taurpc to ttipc";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((flag, files)) if flag == "--write" => write_mode(files),
        Some((path, [])) => preview_mode(path),
        _ => {
            eprintln!("usage: ttipc-migrate <FILE.rs>");
            eprintln!("       ttipc-migrate --write <FILE.rs>...");
            ExitCode::from(2)
        }
    }
}

/// Single file -> stdout, original untouched.
fn preview_mode(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("ttipc-migrate: cannot read {path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    match ttipc_migrate::transform(&src) {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("ttipc-migrate: cannot parse {path}: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Whole project in place, as one commit on a fresh branch.
fn write_mode(files: &[String]) -> ExitCode {
    if files.is_empty() {
        eprintln!("usage: ttipc-migrate --write <FILE.rs>...");
        return ExitCode::from(2);
    }
    match run_write(files) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ttipc-migrate: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_write(files: &[String]) -> Result<(), String> {
    // Read everything up front (absolute paths, so `git -C <root>` resolves them
    // regardless of the working directory).
    let mut inputs: Vec<(PathBuf, String)> = Vec::new();
    for file in files {
        let path = std::fs::canonicalize(file).map_err(|e| format!("cannot read {file}: {e}"))?;
        let src = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {file}: {e}"))?;
        inputs.push((path, src));
    }

    // Transform everything in memory; abort before touching anything on error.
    let registry_input: Vec<(String, String)> = inputs
        .iter()
        .map(|(path, src)| (path.display().to_string(), src.clone()))
        .collect();
    let outputs = ttipc_migrate::transform_project(&registry_input)
        .map_err(|e| format!("cannot parse: {e}"))?;

    // Only the files the migration actually changed.
    let changed: Vec<(&Path, &str)> = inputs
        .iter()
        .enumerate()
        .filter(|(i, (_, src))| *src != outputs[*i].1)
        .map(|(i, (path, _))| (path.as_path(), outputs[i].1.as_str()))
        .collect();
    if changed.is_empty() {
        println!("ttipc-migrate: nothing to migrate");
        return Ok(());
    }

    let root = git_root(&inputs[0].0)?;
    if !git(&root, &["status", "--porcelain"])?.is_empty() {
        return Err("working tree is not clean; commit or stash first".into());
    }
    git(&root, &["checkout", "-b", BRANCH])
        .map_err(|e| format!("could not create branch {BRANCH} (does it exist?): {e}"))?;

    for &(path, out) in &changed {
        std::fs::write(path, out).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    let paths: Vec<String> = changed
        .iter()
        .map(|(p, _)| p.display().to_string())
        .collect();
    let mut add = vec!["add"];
    add.extend(paths.iter().map(String::as_str));
    git(&root, &add)?;
    git(&root, &["commit", "-m", COMMIT_MESSAGE])?;

    println!(
        "ttipc-migrate: migrated {} file(s) on branch {BRANCH}",
        changed.len()
    );
    for &(path, _) in &changed {
        println!("  {}", path.display());
    }
    Ok(())
}

/// The git work-tree root containing `file`.
fn git_root(file: &Path) -> Result<PathBuf, String> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git not available: {e}"))?;
    if !out.status.success() {
        return Err(format!("{} is not in a git repository", file.display()));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Run `git -C <root> <args>`, returning stdout on success or an error with
/// stderr on failure.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
