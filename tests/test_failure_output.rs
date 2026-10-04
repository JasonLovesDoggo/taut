//! Failure reports retain user evidence and offer an exact, executable rerun.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn command(project: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_taut"));
    command
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .env(
            "PYTHONPATH",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("python"),
        );
    command
}

fn run(project: &TempDir, args: &[&str]) -> Output {
    command(project).args(args).output().unwrap()
}

#[cfg(unix)]
fn posix_rerun_command(output: &str) -> &str {
    let reruns: Vec<_> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Rerun (POSIX): "))
        .collect();
    assert_eq!(reruns.len(), 1, "expected exactly one rerun: {output}");
    assert!(reruns[0].contains(" -- "), "{}", reruns[0]);
    reruns[0]
}

#[cfg(unix)]
fn run_posix(project: &TempDir, rerun: &str) -> Output {
    Command::new("/bin/sh")
        .args(["-c", rerun])
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .env(
            "PYTHONPATH",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("python"),
        )
        .output()
        .unwrap()
}

fn failed_stdout(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn assert_contains(output: &str, evidence: &[&str]) {
    for text in evidence {
        assert!(output.contains(text), "missing {text:?} in {output}");
    }
}

fn assert_no_internal_frames(output: &str) {
    assert!(
        !output.contains("File \"<taut worker>\"") && !output.contains("File \"<taut fixtures>\""),
        "focused output retained an embedded frame: {output}"
    );
}

#[test]
fn focused_output_preserves_helpers_captures_and_frame_shaped_user_text() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_evidence.py"),
        r#"import sys

MESSAGE = 'message evidence\n  File "<taut worker>", line 999991, in user_message'
STDOUT = '  File "<taut fixtures>", line 999992, in captured_stdout'
STDERR = '  File "<taut worker>", line 999993, in captured_stderr'
NOTE = 'note evidence\n\nThe above exception was the direct cause of the following exception:\n\nTraceback (most recent call last):\n  File "<taut worker>", line 999994, in user_note\nValueError: note tail'

def evidence_helper():
    error = ValueError(MESSAGE)
    error.add_note(NOTE)
    raise error

def test_evidence():
    print(STDOUT)
    print(STDERR, file=sys.stderr)
    evidence_helper()
"#,
    )
    .unwrap();

    for args in [vec![], vec!["--quiet"]] {
        let stdout = failed_stdout(&run(&project, &args));
        assert_contains(
            &stdout,
            &[
                "in test_evidence",
                "in evidence_helper",
                "evidence_helper()",
                "raise error",
                "message evidence",
                "line 999991, in user_message",
                "line 999992, in captured_stdout",
                "line 999993, in captured_stderr",
                "line 999994, in user_note",
                "ValueError: note tail",
                "Captured stdout",
                "Captured stderr",
                "1 failed",
            ],
        );
        let embedded_file_lines: Vec<_> = stdout
            .lines()
            .filter(|line| {
                line.contains("File \"<taut worker>\"") || line.contains("File \"<taut fixtures>\"")
            })
            .collect();
        assert_eq!(
            embedded_file_lines.len(),
            4,
            "only the four user-provided frame lines should remain: {stdout}"
        );
    }
}

#[test]
fn user_compiled_frames_with_an_embedded_filename_are_preserved() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_dynamic.py"),
        r#"def test_dynamic():
    namespace = {}
    exec(compile("def user_dynamic():\n    raise RuntimeError('dynamic user evidence')\n", "<taut worker>", "exec"), namespace)
    namespace["user_dynamic"]()
"#,
    )
    .unwrap();

    let stdout = failed_stdout(&run(&project, &[]));
    assert_contains(
        &stdout,
        &[
            "in test_dynamic",
            "File \"<taut worker>\", line 2, in user_dynamic",
            "RuntimeError: dynamic user evidence",
        ],
    );
    assert_eq!(
        stdout.matches("File \"<taut worker>\"").count(),
        1,
        "the user-generated frame survives but embedded worker frames do not: {stdout}"
    );
}

#[test]
fn focused_tracebacks_preserve_explicit_and_implicit_exception_chains() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_chains.py"),
        r#"def chain_origin():
    raise ValueError('chain origin evidence')

def test_explicit_cause():
    try:
        chain_origin()
    except ValueError as error:
        raise RuntimeError('explicit wrapper evidence') from error

def test_implicit_context():
    try:
        chain_origin()
    except ValueError:
        raise LookupError('implicit wrapper evidence')
"#,
    )
    .unwrap();

    let stdout = failed_stdout(&run(&project, &["--no-parallel"]));
    assert_no_internal_frames(&stdout);
    assert_contains(
        &stdout,
        &[
            "in chain_origin",
            "chain_origin()",
            "chain origin evidence",
            "explicit wrapper evidence",
            "implicit wrapper evidence",
            "The above exception was the direct cause of the following exception:",
            "During handling of the above exception, another exception occurred:",
            "2 failed",
        ],
    );
}

#[test]
fn focused_tracebacks_preserve_nested_exception_groups_and_leaf_frames() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_groups.py"),
        r#"def group_leaf(message):
    try:
        raise ValueError(message)
    except ValueError as error:
        return error

def test_groups():
    first = group_leaf('first leaf evidence')
    second = group_leaf('nested leaf evidence')
    raise ExceptionGroup('outer group evidence', [first, ExceptionGroup('inner group evidence', [second])])
"#,
    )
    .unwrap();

    let stdout = failed_stdout(&run(&project, &[]));
    assert_no_internal_frames(&stdout);
    assert_contains(
        &stdout,
        &[
            "Exception Group Traceback",
            "in test_groups",
            "in group_leaf",
            "raise ValueError(message)",
            "outer group evidence (2 sub-exceptions)",
            "inner group evidence (1 sub-exception)",
            "ValueError: first leaf evidence",
            "ValueError: nested leaf evidence",
        ],
    );
}

#[test]
fn fixture_setup_and_teardown_keep_user_frames_and_evidence() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_fixtures.py"),
        r#"from taut import fixture

def fixture_origin(message):
    raise RuntimeError(message)

@fixture
def broken_setup():
    fixture_origin('fixture setup evidence')

@fixture
def broken_teardown():
    yield 42
    fixture_origin('fixture teardown evidence')

def test_setup(broken_setup):
    raise AssertionError('setup body must not run')

def test_teardown(broken_teardown):
    print('fixture test body evidence')
    assert broken_teardown == 42
"#,
    )
    .unwrap();

    let stdout = failed_stdout(&run(&project, &["--no-parallel"]));
    assert_no_internal_frames(&stdout);
    assert_contains(
        &stdout,
        &[
            "in broken_setup",
            "in broken_teardown",
            "in fixture_origin",
            "raise RuntimeError(message)",
            "RuntimeError: fixture setup evidence",
            "RuntimeError: fixture teardown evidence",
            "fixture test body evidence",
            "2 failed",
        ],
    );
    assert!(!stdout.contains("setup body must not run"), "{stdout}");
}

#[test]
fn verbose_and_json_retain_raw_tracebacks_without_a_json_schema_change() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_raw.py"),
        "def raw_helper():\n    raise ValueError('raw failure evidence')\ndef test_raw():\n    print('raw capture evidence')\n    raw_helper()\n",
    )
    .unwrap();

    let output = run(&project, &["--json"]);
    failed_stdout(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["summary"]["failed"], 1);
    let error = report["tests"][0]["error"].as_object().unwrap();
    assert_eq!(error.len(), 2, "unexpected JSON error fields: {error:?}");
    assert!(error.contains_key("message"));
    let raw = error["traceback"].as_str().unwrap();
    assert_contains(raw, &["File \"<taut worker>\"", "raw failure evidence"]);
    assert_eq!(report["tests"][0]["stdout"], "raw capture evidence\n");

    let verbose = failed_stdout(&run(&project, &["--verbose"]));
    for line in raw.lines() {
        assert!(verbose.contains(line), "verbose lost {line:?}: {verbose}");
    }
    let focused = failed_stdout(&run(&project, &[]));
    assert_no_internal_frames(&focused);
    assert_contains(&focused, &["raw failure evidence", "raw capture evidence"]);
}

#[cfg(unix)]
#[test]
fn posix_rerun_executes_only_the_selected_parameter_with_quoted_file_and_case_id() {
    let project = TempDir::new().unwrap();
    let filename = "test odd 'quoted' [file].py";
    fs::write(
        project.path().join(filename),
        r#"from pathlib import Path
from taut import parametrize

@parametrize('value', [0, 1, 2], ids=["odd 'case' [zero] $(printf changed) `printf changed`", 'one', 'two'])
def test_value(value):
    with Path('witness.txt').open('a') as witness:
        witness.write(str(value) + '\n')
    assert value != 0, 'selected parameter evidence'
"#,
    )
    .unwrap();
    let initial = failed_stdout(&run(&project, &[filename, "--no-parallel"]));
    assert_contains(&initial, &["2 passed, 1 failed"]);
    let rerun_command = posix_rerun_command(&initial);
    fs::write(project.path().join("witness.txt"), "").unwrap();

    let rerun = failed_stdout(&run_posix(&project, rerun_command));
    assert_contains(
        &rerun,
        &["0 passed, 1 failed", "selected parameter evidence"],
    );
    assert_eq!(
        fs::read_to_string(project.path().join("witness.txt")).unwrap(),
        "0\n",
        "copying the hint must execute exactly its failed parameter"
    );
}

#[cfg(unix)]
#[test]
fn rerun_preserves_explicit_default_async_concurrency_over_project_config() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("pyproject.toml"),
        "[tool.taut]\nasync-concurrency = 2\n",
    )
    .unwrap();
    fs::write(
        project.path().join("test_override.py"),
        "def test_override():\n    raise AssertionError('async override evidence')\n",
    )
    .unwrap();

    let initial = failed_stdout(&run(
        &project,
        &["--no-parallel", "--async-concurrency", "1"],
    ));
    let rerun_command = posix_rerun_command(&initial);
    assert_contains(rerun_command, &["--no-parallel", "--async-concurrency 1"]);
    // Without the explicit default, config overlap conflicts with --no-parallel.
    let rerun = failed_stdout(&run_posix(&project, rerun_command));
    assert_contains(&rerun, &["async override evidence", "0 passed, 1 failed"]);
}

#[cfg(unix)]
#[test]
fn rerun_preserves_explicit_default_isolation_over_project_config() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("pyproject.toml"),
        "[tool.taut]\nisolation = 'process-per-test'\n",
    )
    .unwrap();
    fs::write(
        project.path().join("test_override.py"),
        "def test_override():\n    raise AssertionError('isolation override evidence')\n",
    )
    .unwrap();

    let initial = failed_stdout(&run(&project, &["--isolation", "process-per-run"]));
    let rerun_command = posix_rerun_command(&initial);
    assert_contains(rerun_command, &["--isolation process-per-run"]);
    let rerun = failed_stdout(&run_posix(&project, rerun_command));
    assert_contains(
        &rerun,
        &["isolation override evidence", "0 passed, 1 failed"],
    );
}

#[cfg(unix)]
#[test]
fn rerun_preserves_and_uses_an_explicit_interpreter_with_spaces_and_an_apostrophe() {
    let discovered = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(
        discovered.status.success(),
        "{}",
        String::from_utf8_lossy(&discovered.stderr)
    );
    let real_python = String::from_utf8(discovered.stdout).unwrap();
    let project = TempDir::new().unwrap();
    let selected_python = project.path().join("python user's choice");
    std::os::unix::fs::symlink(real_python.trim(), &selected_python).unwrap();
    fs::write(
        project.path().join("test_interpreter.py"),
        "import sys\nfrom pathlib import Path\ndef test_interpreter():\n    Path('interpreter.txt').write_text(sys.executable)\n    raise AssertionError('selected interpreter evidence')\n",
    )
    .unwrap();

    let initial = failed_stdout(
        &command(&project)
            .arg("--python")
            .arg(&selected_python)
            .output()
            .unwrap(),
    );
    let rerun_command = posix_rerun_command(&initial);
    let expected_argument = format!(
        "--python '{}'",
        selected_python.to_str().unwrap().replace('\'', "'\"'\"'")
    );
    assert_contains(rerun_command, &[&expected_argument]);
    // Framework Python launchers can resolve the symlink before setting
    // sys.executable; the command above must still retain its lexical path.
    let initial_python = fs::read_to_string(project.path().join("interpreter.txt")).unwrap();
    fs::remove_file(project.path().join("interpreter.txt")).unwrap();

    let rerun = failed_stdout(&run_posix(&project, rerun_command));
    assert_contains(
        &rerun,
        &["selected interpreter evidence", "0 passed, 1 failed"],
    );
    assert_eq!(
        fs::read_to_string(project.path().join("interpreter.txt")).unwrap(),
        initial_python,
        "rerun must use the same interpreter as the explicit original selection"
    );
}

#[test]
fn timeout_and_native_worker_failures_keep_actionable_messages() {
    for (source, evidence) in [
        (
            "import time\ndef test_failure():\n    time.sleep(30)\n",
            "Test exceeded timeout",
        ),
        (
            "import os\ndef test_failure():\n    os._exit(9)\n",
            "Python worker exited before returning a result",
        ),
    ] {
        let project = TempDir::new().unwrap();
        fs::write(project.path().join("test_failure.py"), source).unwrap();
        let stdout = failed_stdout(&run(&project, &["--timeout", "0.2"]));
        assert_contains(
            &stdout,
            &[evidence, "test_failure.py", "1 failed", "Rerun ("],
        );
    }
}
