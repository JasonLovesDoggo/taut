//! Interpreter and installed-environment changes must invalidate --changed results.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn real_python() -> PathBuf {
    let output = Command::new(taut::worker_pool::resolve_python(None))
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

fn project() -> TempDir {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_environment.py"),
        "def test_environment(): pass\n",
    )
    .unwrap();
    project
}

fn command(project: &TempDir, python: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_taut"));
    command
        .args(["--changed", "--json", "--python"])
        .arg(python)
        .current_dir(project.path())
        .env("NO_COLOR", "1");
    command
}

fn assert_counts(output: Output, executed: usize, unchanged: usize) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["executed"], executed, "{report}");
    assert_eq!(report["summary"]["unchanged"], unchanged, "{report}");
}

#[test]
fn path_interpreter_changes_and_lost_execute_permission_invalidate_cached_success() {
    let project = project();
    let executable_directory = TempDir::new().unwrap();
    let executable = executable_directory.path().join("taut-test-python");
    let script = "#!/bin/sh\nexec \"$TAUT_TEST_PYTHON\" \"$@\"\n";
    fs::write(&executable, script).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let real_python = real_python();
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(executable_directory.path().to_path_buf())
            .chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap();
    let run = || {
        command(&project, Path::new("taut-test-python"))
            .env("PATH", &path)
            .env("TAUT_TEST_PYTHON", &real_python)
            .output()
            .unwrap()
    };
    assert_counts(run(), 1, 0);
    assert_counts(run(), 0, 1);
    // This executable is outside the source snapshot. Its metadata must be tracked.
    fs::write(&executable, format!("{script}# interpreter update\n")).unwrap();
    assert_counts(run(), 1, 0);
    assert_counts(run(), 0, 1);
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
    let unavailable = run();
    assert_eq!(
        unavailable.status.code(),
        Some(2),
        "cached success must not hide an unavailable interpreter: {}",
        String::from_utf8_lossy(&unavailable.stdout)
    );
}

#[test]
fn custom_virtualenv_metadata_is_tracked_through_its_python_symlink() {
    let project = project();
    let environment = TempDir::new().unwrap();
    let created = Command::new(real_python())
        .args(["-m", "venv", "--without-pip"])
        .arg(environment.path())
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let python = environment.path().join("bin/python");
    assert!(
        fs::symlink_metadata(&python)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let output = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let installed = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .join("taut_example-1.0.dist-info");
    fs::create_dir(&installed).unwrap();
    let metadata = installed.join("METADATA");
    fs::write(&metadata, "Name: taut-example\nVersion: 1.0\n").unwrap();
    let run = || command(&project, &python).output().unwrap();
    assert_counts(run(), 1, 0);
    assert_counts(run(), 0, 1);
    fs::write(&metadata, "Name: taut-example\nVersion: 2.0\n").unwrap();
    assert_counts(run(), 1, 0);
}
