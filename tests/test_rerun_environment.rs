//! Reruns freeze execution settings while allowing temporary environments to renew.
#![cfg(unix)]

use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

fn command(program: impl AsRef<OsStr>, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(directory)
        .env_remove("TAUT_PYTHON")
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .env("NO_COLOR", "1");
    command
}

fn taut(directory: &Path) -> Command {
    command(env!("CARGO_BIN_EXE_taut"), directory)
}

fn shell(directory: &Path, rerun: &str) -> Command {
    let mut command = command("/bin/sh", directory);
    command.args(["-c", rerun]);
    command
}

fn stdout(output: Output, code: i32) -> String {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn rerun(output: &str) -> &str {
    let commands: Vec<_> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Rerun (POSIX): "))
        .collect();
    assert_eq!(commands.len(), 1, "{output}");
    assert!(commands[0].contains(" --no-config "), "{output}");
    commands[0]
}

fn python(directory: &Path) -> PathBuf {
    PathBuf::from(
        stdout(
            command("python3", directory)
                .args(["-c", "import sys; print(sys.executable)"])
                .output()
                .unwrap(),
            0,
        )
        .trim(),
    )
}

#[test]
fn exact_node_reruns_ignore_nested_config_and_preserve_effective_timeout_and_mode() {
    for timeout in [None, Some(2)] {
        for nested_config in [
            "[tool.taut\nmalformed",
            "[tool.taut]\nasync-concurrency = 2\n",
            "[tool.taut]\ntimeout = 0.01\n",
            "[tool.taut]\npython = './missing-python'\n",
        ] {
            let directory = TempDir::new().unwrap();
            let project = directory.path();
            let nested = project.join("nested");
            fs::create_dir(&nested).unwrap();
            let config = if let Some(seconds) = timeout {
                // Also ensure an explicit configuration interpreter survives --no-config.
                std::os::unix::fs::symlink(python(project), project.join("selected-python"))
                    .unwrap();
                format!(
                    "[tool.taut]\ntimeout = {seconds}\nisolation = 'process-per-test'\npython = './selected-python'\n"
                )
            } else {
                "[tool.taut]\n".to_owned()
            };
            fs::write(project.join("pyproject.toml"), config).unwrap();
            fs::write(nested.join("pyproject.toml"), nested_config).unwrap();
            fs::write(
                nested.join("test_failure.py"),
                "import time\ndef test_failure():\n    time.sleep(0.05)\n    raise AssertionError('nested replay evidence')\n",
            )
            .unwrap();
            let original = stdout(
                taut(project).args([".", "--no-parallel"]).output().unwrap(),
                1,
            );
            let hint = rerun(&original);
            assert!(hint.contains("--no-parallel"), "{hint}");
            if timeout.is_some() {
                assert!(hint.contains("--timeout 2"), "{hint}");
                assert!(hint.contains("--isolation process-per-test"), "{hint}");
                assert!(hint.contains("--python"), "{hint}");
                assert!(hint.contains("selected-python"), "{hint}");
            } else {
                assert!(!hint.contains("--timeout"), "{hint}");
                assert!(!hint.contains("--isolation"), "{hint}");
            }
            let replay = stdout(shell(project, hint).output().unwrap(), 1);
            assert!(
                replay.contains("AssertionError: nested replay evidence"),
                "{replay}"
            );
            assert!(replay.contains("0 passed, 1 failed"), "{replay}");
        }
    }
}

#[test]
fn no_config_bypasses_malformed_toml_in_doctor_list_run_and_watch_without_losing_root() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().canonicalize().unwrap();
    fs::create_dir(project.join("nested")).unwrap();
    fs::write(project.join("pyproject.toml"), "[tool.taut\nmalformed").unwrap();
    fs::write(project.join("application.py"), "VALUE = 42\n").unwrap();
    fs::write(
        project.join("nested/test_root.py"),
        "from application import VALUE\ndef test_root():\n    assert VALUE == 42\n",
    )
    .unwrap();
    let doctor: Value = serde_json::from_str(&stdout(
        taut(&project)
            .args(["doctor", "nested", "--no-config", "--json"])
            .output()
            .unwrap(),
        0,
    ))
    .unwrap();
    assert_eq!(
        doctor["project"]["root"],
        project.to_string_lossy().as_ref()
    );
    assert_eq!(
        doctor["project"]["config_path"],
        project.join("pyproject.toml").to_string_lossy().as_ref()
    );
    assert_eq!(doctor["project"]["config_ignored"], true);
    assert_eq!(doctor["execution"]["async_concurrency"], 1);
    assert!(doctor["execution"]["timeout_seconds"].is_null());
    for args in [
        vec!["list", "nested", "--no-config", "--json"],
        vec!["nested", "--no-config", "--json"],
    ] {
        let report: Value =
            serde_json::from_str(&stdout(taut(&project).args(&args).output().unwrap(), 0)).unwrap();
        if args[0] == "list" {
            assert_eq!(report["collected"], 1);
        } else {
            assert_eq!(report["summary"]["passed"], 1);
        }
    }

    let mut child = taut(&project)
        .args(["watch", "nested", "--no-config", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = sender.send(BufReader::new(output).lines().next());
    });
    let first = receiver.recv_timeout(Duration::from_secs(15));
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    let first = first
        .expect("watch did not report")
        .expect("watch exited")
        .unwrap();
    let report: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(
        report["summary"]["passed"],
        1,
        "{report}; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn automatic_taut_python_is_reselected_when_its_previous_path_disappears() {
    let directory = TempDir::new().unwrap();
    let project = directory.path();
    let old = project.join("old-python");
    let renewed = project.join("renewed-python");
    let real = python(project);
    for path in [&old, &renewed] {
        std::os::unix::fs::symlink(&real, path).unwrap();
    }
    fs::write(
        project.join("test_failure.py"),
        "def test_failure():\n    raise AssertionError('renewed interpreter evidence')\n",
    )
    .unwrap();
    let original = stdout(taut(project).env("TAUT_PYTHON", &old).output().unwrap(), 1);
    let hint = rerun(&original);
    assert!(!hint.contains("--python"), "{hint}");
    assert!(
        original.contains("Keep the same uv run prefix or activated environment when rerunning.")
    );
    fs::remove_file(&old).unwrap();
    let replay = stdout(
        shell(project, hint)
            .env("TAUT_PYTHON", &renewed)
            .output()
            .unwrap(),
        1,
    );
    assert!(
        replay.contains("AssertionError: renewed interpreter evidence"),
        "{replay}"
    );
}

#[test]
fn uv_overlay_rerun_recreates_the_environment_and_retains_local_dependencies() {
    if !Command::new("uv")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        eprintln!("uv unavailable; skipping local overlay integration");
        return;
    }
    let directory = TempDir::new().unwrap();
    let project = directory.path();
    let wheel = project.join("taut_overlay_probe-0.0.0-py3-none-any.whl");
    let real = python(project);
    stdout(
        command(&real, project)
            .args([
                "-c",
                r#"import sys, zipfile
metadata = 'taut_overlay_probe-0.0.0.dist-info/'
files = {
    'taut_overlay_probe/__init__.py': "VALUE = 'local overlay dependency'\n",
    metadata + 'METADATA': 'Metadata-Version: 2.1\nName: taut-overlay-probe\nVersion: 0.0.0\n',
    metadata + 'WHEEL': 'Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n',
}
files[metadata + 'RECORD'] = ''.join(name + ',,\n' for name in files) + metadata + 'RECORD,,\n'
with zipfile.ZipFile(sys.argv[1], 'w') as wheel:
    for name, data in files.items():
        wheel.writestr(name, data)
"#,
            ])
            .arg(&wheel)
            .output()
            .unwrap(),
        0,
    );
    fs::write(
        project.join("test_overlay.py"),
        "import json, os, sys\nfrom pathlib import Path\nimport taut_overlay_probe\ndef test_overlay():\n    Path('environment.json').write_text(json.dumps({'venv': os.environ['VIRTUAL_ENV'], 'python': sys.executable, 'dependency': taut_overlay_probe.VALUE}))\n    raise AssertionError(taut_overlay_probe.VALUE)\n",
    )
    .unwrap();
    let uv = || {
        let mut command = command("uv", project);
        command
            .args([
                "run",
                "--no-project",
                "--no-config",
                "--offline",
                "--no-cache",
                "--python",
            ])
            .arg(&real)
            .arg("--with")
            .arg(&wheel)
            .env("UV_CACHE_DIR", project.join("isolated-uv-cache"))
            .env("UV_PYTHON_DOWNLOADS", "never")
            .env("UV_NO_PROGRESS", "1");
        command
    };
    let original = stdout(
        uv().args([env!("CARGO_BIN_EXE_taut"), "test_overlay.py"])
            .output()
            .unwrap(),
        1,
    );
    let witness = || -> Value {
        serde_json::from_slice(&fs::read(project.join("environment.json")).unwrap()).unwrap()
    };
    let first = witness();
    assert_eq!(first["dependency"], "local overlay dependency");
    for key in ["venv", "python"] {
        assert!(
            !Path::new(first[key].as_str().unwrap()).exists(),
            "uv retained {key}"
        );
    }
    let hint = rerun(&original);
    assert!(!hint.contains("--python"), "{hint}");
    assert!(
        original.contains("Keep the same uv run prefix or activated environment when rerunning.")
    );
    fs::remove_file(project.join("environment.json")).unwrap();
    let replay = stdout(uv().args(["/bin/sh", "-c", hint]).output().unwrap(), 1);
    assert!(
        replay.contains("AssertionError: local overlay dependency"),
        "{replay}"
    );
    let second = witness();
    assert_eq!(second["dependency"], "local overlay dependency");
    assert_ne!(first["venv"], second["venv"]);
    assert!(!Path::new(second["python"].as_str().unwrap()).exists());
}
