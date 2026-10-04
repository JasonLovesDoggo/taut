//! The environment reported by doctor must be the environment that executes tests.
#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

fn real_python() -> PathBuf {
    let output = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

fn root(directory: &TempDir) -> PathBuf {
    directory.path().canonicalize().unwrap()
}

fn command(binary: &Path, directory: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(directory)
        .env_remove("TAUT_PYTHON")
        .env_remove("VIRTUAL_ENV")
        .env("NO_COLOR", "1");
    command
}

fn taut(directory: &Path) -> Command {
    command(Path::new(env!("CARGO_BIN_EXE_taut")), directory)
}

fn report(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)))
}

fn executable(path: &Path, source: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn forwarding_python(path: &Path) {
    executable(path, "#!/bin/sh\nexec \"$TAUT_TEST_REAL_PYTHON\" \"$@\"\n");
}

fn copied_launcher(directory: &Path) -> PathBuf {
    let binary = directory.join("bin/taut");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_taut"), &binary).unwrap();
    binary
}

fn assert_python(report: &Value, path: &Path, source: &str) {
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["python"]["path"], path.to_string_lossy().as_ref());
    assert_eq!(report["python"]["source"], source);
    assert!(
        report["python"]["version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "{report}"
    );
}

#[test]
fn interpreter_precedence_reports_each_selected_lexical_path() {
    let directory = TempDir::new().unwrap();
    let project = root(&directory);
    let binary = copied_launcher(&project.join("installation"));
    let cli = project.join("cli-python");
    let configured = project.join("tools/config-python");
    let environment = project.join("environment-python");
    let activated = project.join("activated/bin/python");
    let local = project.join(".venv/bin/python");
    let launcher = binary.parent().unwrap().join("python");
    let fallback = project.join("path-bin/python3");
    for path in [
        &cli,
        &configured,
        &environment,
        &activated,
        &local,
        &launcher,
        &fallback,
    ] {
        forwarding_python(path);
    }
    fs::write(
        project.join("pyproject.toml"),
        "[tool.taut]\npython = 'tools/config-python'\n",
    )
    .unwrap();
    let python = real_python();
    let run = |cli_override: bool, taut_override: bool, activated_override: bool| {
        let mut run = command(&binary, &project);
        run.args(["doctor", "--json"])
            .env("TAUT_TEST_REAL_PYTHON", &python)
            .env("PATH", fallback.parent().unwrap());
        if cli_override {
            run.args(["--python", "./cli-python"]);
        }
        if taut_override {
            run.env("TAUT_PYTHON", &environment);
        }
        if activated_override {
            run.env("VIRTUAL_ENV", activated.parent().unwrap().parent().unwrap());
        }
        report(run.output().unwrap())
    };
    assert_python(&run(true, true, true), &cli, "cli");
    assert_python(&run(false, true, true), &configured, "config");
    fs::write(
        project.join("pyproject.toml"),
        "[project]\nname = 'example'\n",
    )
    .unwrap();
    assert_python(&run(false, true, true), &environment, "taut_python");
    assert_python(&run(false, false, true), &activated, "virtual_env");
    assert_python(&run(false, false, false), &local, "project_venv");
    fs::remove_dir_all(project.join(".venv")).unwrap();
    assert_python(&run(false, false, false), &launcher, "launcher");
    fs::remove_file(&launcher).unwrap();
    assert_python(&run(false, false, false), &fallback, "path");
}

#[test]
fn external_project_venv_beats_launcher_for_doctor_normal_changed_and_watch() {
    let directory = TempDir::new().unwrap();
    let base = root(&directory);
    let caller = base.join("caller/deep/nested");
    let project = base.join("selected");
    let tests = project.join("tests/unit");
    fs::create_dir_all(&caller).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(
        project.join("pyproject.toml"),
        "[project]\nname = 'selected'\n",
    )
    .unwrap();
    let binary = copied_launcher(&base.join("installation"));
    executable(
        &binary.parent().unwrap().join("python"),
        "#!/bin/sh\necho 'launcher Python must not run' >&2\nexit 91\n",
    );
    executable(
        &base.join("caller/.venv/bin/python"),
        "#!/bin/sh\necho 'cwd Python must not run' >&2\nexit 92\n",
    );
    let venv = project.join(".venv");
    let created = Command::new(real_python())
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let selected = tests.join("test_environment.py");
    fs::write(
        &selected,
        "import os, sys\ndef test_environment():\n    assert sys.prefix == os.environ['TAUT_EXPECTED_PREFIX']\n    assert sys.prefix != sys.base_prefix\n",
    )
    .unwrap();
    let selection = format!("{}::test_environment", selected.display());
    let diagnosis = report(
        command(&binary, &caller)
            .args(["doctor", &selection, "--json"])
            .output()
            .unwrap(),
    );
    assert_python(&diagnosis, &venv.join("bin/python"), "project_venv");
    assert_eq!(
        diagnosis["project"]["root"],
        project.to_string_lossy().as_ref()
    );
    for (changed, expected_executed) in [(false, 1), (true, 1), (true, 0)] {
        let mut run = command(&binary, &caller);
        run.args([&selection, "--json"])
            .env("TAUT_EXPECTED_PREFIX", &venv);
        if changed {
            run.arg("--changed");
        }
        let result = report(run.output().unwrap());
        assert_eq!(result["summary"]["executed"], expected_executed, "{result}");
        assert_eq!(result["summary"]["failed"], 0, "{result}");
        assert_eq!(
            result["summary"]["unchanged"],
            1 - expected_executed,
            "{result}"
        );
    }
    let mut child = command(&binary, &caller)
        .args(["watch", &selection, "--json"])
        .env("TAUT_EXPECTED_PREFIX", &venv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let first = BufReader::new(stdout).lines().next();
        let _ = sender.send(first);
    });
    let first = receiver.recv_timeout(Duration::from_secs(15));
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    let first = first
        .expect("watch did not produce its initial report")
        .expect("watch produced no report")
        .unwrap();
    let result: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(
        result["summary"]["passed"],
        1,
        "{result}; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn first_selected_project_governs_interpreter_and_isolation_for_all_paths() {
    let directory = TempDir::new().unwrap();
    let base = root(&directory);
    let python = real_python();
    let projects = [base.join("first"), base.join("second")];
    for (index, project) in projects.iter().enumerate() {
        let venv = project.join(".venv");
        let created = Command::new(&python)
            .args(["-m", "venv", "--without-pip"])
            .arg(&venv)
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let isolation = if index == 0 {
            "process-per-run"
        } else {
            "process-per-test"
        };
        fs::write(
            project.join("pyproject.toml"),
            format!(
                "[tool.taut]\npython = '.venv/bin/python'\nmax-workers = 1\nisolation = '{isolation}'\n"
            ),
        )
        .unwrap();
        fs::write(
            project.join(format!("test_project_{index}.py")),
            format!(
                "import os, sys\nfrom pathlib import Path\ndef test_environment():\n    assert sys.prefix == os.environ['TAUT_EXPECTED_PREFIX']\n    assert sys.prefix != sys.base_prefix\n    Path(os.environ['TAUT_PID_DIRECTORY'], '{index}').write_text(str(os.getpid()))\n"
            ),
        )
        .unwrap();
    }

    for (first, second) in [(0, 1), (1, 0)] {
        let pid_directory = base.join(format!("pids-{first}"));
        fs::create_dir(&pid_directory).unwrap();
        let result = report(
            taut(&base)
                .arg(&projects[first])
                .arg(&projects[second])
                .arg("--json")
                .env("TAUT_EXPECTED_PREFIX", projects[first].join(".venv"))
                .env("TAUT_PID_DIRECTORY", &pid_directory)
                .output()
                .unwrap(),
        );
        assert_eq!(result["summary"]["collected"], 2, "{result}");
        assert_eq!(result["summary"]["executed"], 2, "{result}");
        assert_eq!(result["summary"]["passed"], 2, "{result}");
        let first_pid = fs::read_to_string(pid_directory.join("0")).unwrap();
        let second_pid = fs::read_to_string(pid_directory.join("1")).unwrap();
        assert_eq!(
            first_pid == second_pid,
            first == 0,
            "the first project's isolation setting must govern both selected projects"
        );
    }
}

#[test]
fn nearest_project_marker_bounds_config_and_doctor_has_no_project_side_effects() {
    let directory = TempDir::new().unwrap();
    let outer = root(&directory);
    fs::write(
        outer.join("pyproject.toml"),
        "[tool.taut]\npython = '/must/not/inherit/python'\nmax-workers = 99\n",
    )
    .unwrap();
    let project = outer.join("subpackage");
    let nested = project.join("tests/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(project.join("pytest.ini"), "[pytest]\n").unwrap();
    let imported = project.join("imported.txt");
    let source = format!(
        "from pathlib import Path\nPath({:?}).touch()\nraise RuntimeError('doctor imported project code')\n",
        imported.to_str().unwrap()
    );
    fs::write(project.join("conftest.py"), &source).unwrap();
    fs::write(nested.join("test_side_effect.py"), &source).unwrap();
    // Doctor must not collect Python either: a broken test remains diagnosable.
    fs::write(nested.join("test_invalid.py"), "def broken(:\n").unwrap();
    let python = project.join(".venv/bin/python");
    forwarding_python(&python);
    let result = report(
        taut(&nested)
            .args(["doctor", "--json"])
            .env("TAUT_TEST_REAL_PYTHON", real_python())
            .output()
            .unwrap(),
    );
    assert_python(&result, &python, "project_venv");
    assert_eq!(
        result["project"]["root"],
        project.to_string_lossy().as_ref()
    );
    assert!(result["project"]["config_path"].is_null(), "{result}");
    assert!(!imported.exists());
    for path in [&project, &nested] {
        assert!(!path.join("__pycache__").exists());
        assert!(!path.join(".taut").exists());
    }
    let cache = report(
        taut(&nested)
            .args(["cache", "info", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        cache["cache"]["exists"], false,
        "doctor created a dependency cache: {cache}"
    );
}

#[test]
fn doctor_reports_configured_settings_and_effective_cli_overrides() {
    let directory = TempDir::new().unwrap();
    let project = root(&directory);
    forwarding_python(&project.join("tools/python"));
    fs::write(
        project.join("pyproject.toml"),
        "[tool.taut]\npython = 'tools/python'\nmax-workers = 3\nisolation = 'process-per-run'\nasync-concurrency = 4\ntimeout = 2.5\nfail-fast = true\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        report(
            taut(&project)
                .args(["doctor", "--json"])
                .args(args)
                .env("TAUT_TEST_REAL_PYTHON", real_python())
                .output()
                .unwrap(),
        )
    };
    let configured = run(&[]);
    assert_python(&configured, &project.join("tools/python"), "config");
    assert_eq!(
        configured["project"]["config_path"],
        project.join("pyproject.toml").to_string_lossy().as_ref()
    );
    assert_eq!(
        configured["execution"],
        serde_json::json!({
            "parallel": true, "jobs": 3, "isolation": "process-per-run",
            "async_concurrency": 4, "timeout_seconds": 2.5,
            "fail_fast": true, "changed": false, "filter": null
        })
    );
    let overridden = run(&[
        "--no-parallel",
        "--jobs",
        "8",
        "--isolation",
        "process-per-test",
        "--async-concurrency",
        "1",
        "--timeout",
        "7",
        "--changed",
        "-k",
        "environment",
    ]);
    assert_eq!(
        overridden["execution"],
        serde_json::json!({
            "parallel": false, "jobs": 1, "isolation": "process-per-test",
            "async_concurrency": 1, "timeout_seconds": 7.0,
            "fail_fast": true, "changed": true, "filter": "environment"
        })
    );
}

#[test]
fn invalid_python_and_configuration_are_machine_readable_setup_errors() {
    let directory = TempDir::new().unwrap();
    let project = root(&directory);
    let missing = project.join("missing-python");
    let rejected = project.join("rejected-python");
    executable(&rejected, "#!/bin/sh\necho 'probe rejected' >&2\nexit 17\n");
    for python in [&missing, &rejected] {
        let output = taut(&project)
            .args(["doctor", "--json", "--python"])
            .arg(python)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["schema_version"], 1);
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains(python.file_name().unwrap().to_str().unwrap()),
            "{result}"
        );
    }
    let optimized = taut(&project)
        .args(["doctor", "--json", "--python"])
        .arg(real_python())
        .env("PYTHONOPTIMIZE", "1")
        .output()
        .unwrap();
    assert_eq!(optimized.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&optimized.stdout).unwrap();
    assert_eq!(result["schema_version"], 1);
    assert!(
        result["error"].as_str().unwrap().contains("optimization"),
        "optimized Python must not disable assertions silently: {result}"
    );
    for configuration in ["[tool.taut\n", "[tool.taut]\nmax-workers = 0\n"] {
        fs::write(project.join("pyproject.toml"), configuration).unwrap();
        let output = taut(&project).args(["doctor", "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["schema_version"], 1);
        assert!(
            result["error"].as_str().unwrap().contains("pyproject.toml"),
            "{result}"
        );
    }
}

#[test]
fn venv_only_project_stops_outer_fixtures_at_the_reported_root() {
    let directory = TempDir::new().unwrap();
    let outer = root(&directory);
    let project = outer.join("inner");
    let nested = project.join("tests/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(outer.join("pyproject.toml"), "[tool.taut]\n").unwrap();
    fs::write(
        outer.join("conftest.py"),
        "raise RuntimeError('outer conftest crossed the project boundary')\n",
    )
    .unwrap();
    fs::write(
        project.join("conftest.py"),
        "def number(): return 42\nnumber.__taut_fixture__ = {}\n",
    )
    .unwrap();
    fs::write(
        nested.join("test_boundary.py"),
        "def test_number(number): assert number == 42\ndef test_plain(): pass\n",
    )
    .unwrap();
    let python = project.join(".venv/bin/python");
    forwarding_python(&python);
    let interpreter = real_python();
    let diagnose = report(
        taut(&nested)
            .args(["doctor", "--json"])
            .env("TAUT_TEST_REAL_PYTHON", &interpreter)
            .output()
            .unwrap(),
    );
    assert_eq!(
        diagnose["project"]["root"],
        project.to_string_lossy().as_ref()
    );
    assert!(diagnose["project"]["config_path"].is_null());
    assert_python(&diagnose, &python, "project_venv");
    for args in [vec!["--json"], vec!["--changed", "--json"]] {
        let result = report(
            taut(&nested)
                .args(args)
                .env("TAUT_TEST_REAL_PYTHON", &interpreter)
                .output()
                .unwrap(),
        );
        assert_eq!(result["summary"]["passed"], 2, "{result}");
    }
}
