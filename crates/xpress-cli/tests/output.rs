//! `-o` with several inputs: never a silent in-place fallback, never two
//! inputs written to the same output.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const PNG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../xpress-core/tests/fixtures/ocr.png"
);

fn xpress(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xpress"))
        .current_dir(cwd)
        .args(args)
        .env("HOME", cwd)
        .output()
        .unwrap()
}

#[test]
fn missing_output_folder_is_created_not_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::copy(PNG, d.join("a.png")).unwrap();
    fs::copy(PNG, d.join("b.png")).unwrap();
    let before = fs::read(d.join("a.png")).unwrap();

    let out = xpress(
        d,
        &["optimise", "--no-backup", "-o", "dist", "a.png", "b.png"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(d.join("dist").is_dir());
    // The sources are untouched.
    assert_eq!(fs::read(d.join("a.png")).unwrap(), before);
}

#[test]
fn output_that_is_a_file_is_an_error_with_several_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::copy(PNG, d.join("a.png")).unwrap();
    fs::copy(PNG, d.join("b.png")).unwrap();
    fs::write(d.join("dist"), b"not a folder").unwrap();
    let out = xpress(
        d,
        &["optimise", "--no-backup", "-o", "dist", "a.png", "b.png"],
    );
    assert!(!out.status.success());
}

#[test]
fn same_name_in_two_folders_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::create_dir_all(d.join("src/a")).unwrap();
    fs::create_dir_all(d.join("src/b")).unwrap();
    fs::copy(PNG, d.join("src/a/hero.png")).unwrap();
    fs::copy(PNG, d.join("src/b/hero.png")).unwrap();
    let out = xpress(d, &["optimise", "-r", "--no-backup", "-o", "dist", "src"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("hero.png"));
    assert!(!d.join("dist/hero.png").exists());
}
