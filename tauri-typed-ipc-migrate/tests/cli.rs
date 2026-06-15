//! The `ttipc-migrate` binary: a file path in, ttipc source on stdout,
//! a non-zero exit and a stderr message on failure.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_ttipc-migrate");

const INPUT: &str = r#"
#[taurpc::procedures(path = "greeter")]
pub trait Greeter {
    async fn greet<R: Runtime>(app_handle: AppHandle<R>, name: String) -> Result<String>;
}
"#;

#[test]
fn transforms_a_file_to_stdout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("greeter.rs");
    std::fs::write(&path, INPUT).expect("write input");

    let output = Command::new(BIN).arg(&path).output().expect("run bin");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("#[ttipc::procedures(path = \"greeter\")]"),
        "got:\n{stdout}"
    );
    assert!(
        stdout.contains("async fn greet(&self, app_handle: AppHandle, name: String)"),
        "got:\n{stdout}"
    );
    assert!(
        stdout.contains("Manual follow-ups"),
        "header missing:\n{stdout}"
    );
}

#[test]
fn missing_file_is_an_error() {
    let output = Command::new(BIN)
        .arg("does-not-exist.rs")
        .output()
        .expect("run bin");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot read"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn no_argument_is_a_usage_error() {
    let output = Command::new(BIN).output().expect("run bin");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("usage"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("run git")
        .success();
    assert!(ok, "git {args:?} failed");
}

#[test]
fn write_mode_migrates_in_place_on_a_branch() {
    // A two-file project where the trigger is declared in one file and emitted in
    // another: `--write` builds a project-wide registry, edits both files in
    // place, and lands the change as one commit on a fresh branch.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let trait_path = root.join("cmd.rs");
    let emit_path = root.join("store.rs");
    std::fs::write(
        &trait_path,
        "#[taurpc::procedures(path = \"cmd\", event_trigger = CmdBus)]\n\
         pub trait Cmd {\n    #[taurpc(event)]\n    async fn updated(value: u8);\n}\n",
    )
    .expect("write trait");
    std::fs::write(
        &emit_path,
        "// keep me\nimpl Store {\n    fn touch(&self, app: AppHandle, value: u8) {\n        \
         CmdBus::new(app).updated(value).unwrap();\n    }\n}\n",
    )
    .expect("write emit");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "init"]);

    let output = Command::new(BIN)
        .arg("--write")
        .arg(&trait_path)
        .arg(&emit_path)
        .output()
        .expect("run bin");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // On the migration branch now.
    let branch = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("git branch");
    assert_eq!(
        String::from_utf8_lossy(&branch.stdout).trim(),
        "ttipc-migration"
    );

    // The emit file changed only its emit site (the comment is preserved); the
    // cross-file trigger resolved to the generated enum.
    let emit = std::fs::read_to_string(&emit_path).expect("read emit");
    assert!(emit.contains("// keep me"), "comment lost:\n{emit}");
    assert!(
        emit.contains("CmdEvent::Updated { value: value }.emit(&app).unwrap()")
            && !emit.contains("CmdBus::new"),
        "emit not rewritten:\n{emit}"
    );
    let cmd = std::fs::read_to_string(&trait_path).expect("read trait");
    assert!(
        cmd.contains("#[derive(ttipc::Event)]") && cmd.contains("enum CmdEvent"),
        "event enum missing:\n{cmd}"
    );

    // It is a single commit, and the tree is clean (everything committed).
    let porcelain = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
        .expect("git status");
    assert!(
        porcelain.stdout.is_empty(),
        "tree not clean: {}",
        String::from_utf8_lossy(&porcelain.stdout)
    );
}
