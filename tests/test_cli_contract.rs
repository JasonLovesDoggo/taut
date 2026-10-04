//! End-to-end CLI guarantees: execution, diagnostics and machine-readable output.
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(project: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taut"))
        .args(args)
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)))
}

#[test]
fn defaults_execute_tests_on_every_invocation() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("test_runs.py"), "from pathlib import Path\ndef test_runs():\n    path = Path('counter.txt')\n    count = int(path.read_text()) if path.exists() else 0\n    path.write_text(str(count + 1))\n").unwrap();
    for _ in 0..2 {
        let result = run(&project, &["--json"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(json(&result)["summary"]["executed"], 1);
        assert_eq!(json(&result)["summary"]["unchanged"], 0);
    }
    assert_eq!(
        fs::read_to_string(project.path().join("counter.txt")).unwrap(),
        "2"
    );
}

#[test]
fn json_captures_test_output_without_contaminating_the_document() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("test_output.py"), "import sys\ndef test_output():\n    print('diagnostic stdout')\n    print('diagnostic stderr', file=sys.stderr)\n    assert False, 'useful assertion'\n").unwrap();
    let result = run(&project, &["--json"]);
    assert_eq!(result.status.code(), Some(1));
    let report = json(&result);
    assert_eq!(report["summary"]["failed"], 1);
    assert!(
        report["tests"][0]["stdout"]
            .as_str()
            .unwrap()
            .contains("diagnostic stdout")
    );
    assert!(
        report["tests"][0]["stderr"]
            .as_str()
            .unwrap()
            .contains("diagnostic stderr")
    );
    assert!(
        report["tests"][0]["error"]["traceback"]
            .as_str()
            .unwrap()
            .contains("useful assertion")
    );
}

#[test]
fn all_human_output_modes_show_complete_failure_evidence() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("test_output.py"), "import sys\ndef helper():\n    raise ValueError('inside helper')\ndef test_output():\n    print('captured breadcrumb')\n    print('stderr breadcrumb', file=sys.stderr)\n    helper()\n").unwrap();
    for args in [vec![], vec!["-v"], vec!["-q"]] {
        let result = run(&project, &args);
        assert_eq!(result.status.code(), Some(1));
        let stdout = String::from_utf8_lossy(&result.stdout);
        for text in [
            "inside helper",
            "captured breadcrumb",
            "stderr breadcrumb",
            "helper()",
            "1 failed",
        ] {
            assert!(stdout.contains(text), "missing {text} in {stdout}");
        }
    }
}

#[test]
fn usage_configuration_and_empty_suites_have_distinct_exit_codes() {
    let project = TempDir::new().unwrap();
    for args in [
        vec!["--jobs", "0"],
        vec!["--isolation", "typo"],
        vec!["--timeout", "inf"],
        vec!["--async-concurrency", "0"],
        vec!["missing.py"],
    ] {
        assert_eq!(run(&project, &args).status.code(), Some(2));
    }
    assert_eq!(run(&project, &[]).status.code(), Some(5));
    assert_eq!(run(&project, &["list"]).status.code(), Some(5));
    assert_eq!(run(&project, &["--help"]).status.code(), Some(0));
    assert_eq!(run(&project, &["--version"]).status.code(), Some(0));
    fs::write(
        project.path().join("pyproject.toml"),
        "[tool.taut]\nmax_workers = -1",
    )
    .unwrap();
    let result = run(&project, &["--json"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        json(&result)["error"]
            .as_str()
            .unwrap()
            .contains("pyproject.toml")
    );
}

#[test]
fn list_and_execution_share_exact_node_selection() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_select.py"),
        "def test_one(): pass\ndef test_two(): raise AssertionError('must not execute')\n",
    )
    .unwrap();
    let result = run(&project, &["list", "test_select.py::test_one", "--json"]);
    assert_eq!(result.status.code(), Some(0));
    assert_eq!(json(&result)["collected"], 1);
    let result = run(&project, &["test_select.py::test_one", "--json"]);
    assert_eq!(result.status.code(), Some(0));
    assert_eq!(json(&result)["summary"]["executed"], 1);
}

#[test]
fn quiet_suppresses_progress_but_retains_summary() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_pass.py"),
        "def test_pass(): pass\n",
    )
    .unwrap();
    let result = run(&project, &["-q"]);
    assert_eq!(result.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("1 passed"));
}

#[test]
fn default_execution_has_no_dependency_tracing() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_trace.py"),
        "import sys\ndef test_trace():\n    assert sys.gettrace() is None\n",
    )
    .unwrap();
    assert_eq!(run(&project, &[]).status.code(), Some(0));
}

#[test]
fn fail_fast_distinguishes_unstarted_from_skipped_tests() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("test_stop.py"), "from pathlib import Path\ndef test_first():\n    assert False\ndef test_second():\n    Path('ran_second').touch()\n").unwrap();
    let result = run(&project, &["-x", "-j", "1", "--json"]);
    assert_eq!(result.status.code(), Some(1));
    let report = json(&result);
    assert_eq!(report["summary"]["executed"], 1);
    assert_eq!(report["summary"]["not_run"], 1);
    assert_eq!(report["summary"]["skipped"], 0);
    assert!(!project.path().join("ran_second").exists());
}

#[test]
fn missing_interpreter_is_a_setup_error() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_pass.py"),
        "def test_pass(): pass\n",
    )
    .unwrap();
    let result = run(&project, &["--python", "/does/not/exist/python", "--json"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        json(&result)["error"]
            .as_str()
            .unwrap()
            .contains("/does/not/exist/python")
    );
}

#[test]
fn incompatible_concurrency_settings_are_rejected() {
    let project = TempDir::new().unwrap();
    for args in [
        vec!["--no-parallel", "--async-concurrency", "2", "--json"],
        vec![
            "--isolation",
            "process-per-test",
            "--async-concurrency",
            "2",
            "--json",
        ],
    ] {
        let result = run(&project, &args);
        assert_eq!(result.status.code(), Some(2));
        assert!(
            json(&result)["error"]
                .as_str()
                .unwrap()
                .contains("async-concurrency")
        );
    }
}
