//! Selection diagnostics stay static and preserve the CLI's machine contract.
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(project: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taut"))
        .args(args)
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .env("VIRTUAL_ENV", project.path().join("missing-environment"))
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn help_teaches_common_commands_and_groups_existing_flags() {
    let project = TempDir::new().unwrap();
    let output = run(&project, &["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for text in [
        "Common commands:",
        "taut tests/test_api.py::test_get",
        "taut -k 'login*'",
        "taut list tests",
        "taut watch tests",
        "Selection:",
        "Execution:",
        "Output:",
        "Case-insensitive name substring/glob",
        "no boolean expressions",
        "5 no tests collected",
    ] {
        assert!(help.contains(text), "missing {text:?} in {help}");
    }
    assert!(!help.contains("name expression"));
}

#[test]
fn empty_discovery_reports_paths_and_filename_rules_without_running_python() {
    let project = TempDir::new().unwrap();
    fs::create_dir(project.path().join("tests")).unwrap();
    fs::write(
        project.path().join("tests/example.py"),
        "raise RuntimeError('must not import')\ndef test_hidden(): pass\n",
    )
    .unwrap();
    for prefix in [vec![], vec!["list"]] {
        let mut args = prefix;
        args.extend(["tests", "--python", "/does/not/exist/python", "-k", "typo"]);
        let output = run(&project, &args);
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stderr.is_empty());
        let message = String::from_utf8_lossy(&output.stdout);
        for text in [
            "No tests discovered.",
            "Searched: tests",
            "test_*.py, _test*.py, or *_test.py",
            "test_* or _test* functions",
            "Pass a Python file explicitly",
            "taut list -- tests",
        ] {
            assert!(message.contains(text), "missing {text:?} in {message}");
        }
        assert!(!message.contains("No tests matched filter"));
    }
}

#[test]
fn typo_filter_reports_discovery_count_and_unfiltered_list_hint() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_auth.py"),
        "raise RuntimeError('must not import')\ndef test_login(): pass\ndef test_logout(): pass\n",
    )
    .unwrap();
    for prefix in [vec![], vec!["list"]] {
        let mut args = prefix;
        args.extend([
            "test_auth.py",
            "-k",
            "logni",
            "--python",
            "/does/not/exist/python",
        ]);
        let output = run(&project, &args);
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stderr.is_empty());
        let message = String::from_utf8_lossy(&output.stdout);
        assert!(message.contains("No tests matched filter \"logni\""));
        assert!(message.contains("2 tests before filtering"));
        assert!(message.contains("Searched: test_auth.py"));
        assert!(message.contains("taut list -- test_auth.py"));
        assert!(!message.contains("No tests discovered"));
    }
}

#[test]
fn filter_matching_stays_case_insensitive_substring_glob_and_literal_boolean_text() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test_auth.py"),
        "raise RuntimeError('must not import')\ndef test_login(): pass\ndef test_logout(): pass\n",
    )
    .unwrap();
    for (filter, expected) in [("LOGIN", 1), ("log?*", 2), ("login or logout", 0)] {
        let output = run(&project, &["list", "-k", filter, "--json"]);
        assert_eq!(
            output.status.code(),
            Some(if expected == 0 { 5 } else { 0 })
        );
        assert_eq!(json(&output)["collected"], expected);
    }
}

#[test]
fn empty_and_filtered_json_keep_the_existing_schema_and_exit_five() {
    let project = TempDir::new().unwrap();
    for populated in [false, true] {
        if populated {
            fs::write(
                project.path().join("test_auth.py"),
                "def test_login(): pass\n",
            )
            .unwrap();
        }
        let list = run(&project, &["list", "-k", "typo", "--json"]);
        assert_eq!(list.status.code(), Some(5));
        assert_eq!(
            json(&list),
            serde_json::json!({"schema_version": 1, "tests": [], "collected": 0})
        );
        let execution = run(&project, &["-k", "typo", "--json"]);
        assert_eq!(execution.status.code(), Some(5));
        let mut report = json(&execution);
        assert!(report["summary"]["duration_seconds"].as_f64().unwrap() >= 0.0);
        report["summary"]["duration_seconds"] = serde_json::json!(0);
        assert_eq!(
            report,
            serde_json::json!({
                "schema_version": 1,
                "summary": {"collected": 0, "executed": 0, "passed": 0, "failed": 0,
                    "skipped": 0, "unchanged": 0, "not_run": 0, "duration_seconds": 0},
                "tests": [], "selected_out": []
            })
        );
    }
}

#[test]
fn unmatched_exact_node_has_bounded_ids_and_a_shell_safe_list_hint() {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("test auth's.py"),
        (0..9)
            .map(|number| format!("def test_{number}(): pass\n"))
            .collect::<String>(),
    )
    .unwrap();
    for json_mode in [false, true] {
        let mut args = vec!["list", "test auth's.py::test_typo"];
        if json_mode {
            args.push("--json");
        }
        let output = run(&project, &args);
        assert_eq!(output.status.code(), Some(2));
        let message = if json_mode {
            let report = json(&output);
            assert_eq!(report.as_object().unwrap().len(), 2);
            assert_eq!(report["schema_version"], 1);
            report["error"].as_str().unwrap().to_owned()
        } else {
            assert!(output.stdout.is_empty());
            String::from_utf8(output.stderr).unwrap()
        };
        assert!(message.contains("Positional node IDs must match exactly"));
        let hint = if cfg!(windows) {
            "taut list -- 'test auth''s.py'"
        } else {
            "taut list -- 'test auth'\\''s.py'"
        };
        assert!(message.contains(hint));
        assert_eq!(
            message
                .lines()
                .filter(|line| line.starts_with("  test auth's.py::"))
                .count(),
            5
        );
    }
}

#[test]
fn syntax_errors_report_unicode_character_columns_and_source_carets() {
    let project = TempDir::new().unwrap();
    for newline in ["\n", "\r\n", "\r"] {
        fs::write(
            project.path().join("test_unicode.py"),
            format!("# café ☃{newline}def test_é(:{newline}    pass{newline}"),
        )
        .unwrap();
        for json_mode in [false, true] {
            let mut args = vec!["list", "test_unicode.py", "-k", "no_matches"];
            if json_mode {
                args.push("--json");
            }
            let output = run(&project, &args);
            assert_eq!(output.status.code(), Some(2));
            let message = if json_mode {
                json(&output)["error"].as_str().unwrap().to_owned()
            } else {
                assert!(output.stdout.is_empty());
                String::from_utf8(output.stderr).unwrap()
            };
            assert!(
                message.contains("CollectionSyntaxError: test_unicode.py:2:12:"),
                "{message}"
            );
            assert!(
                message.contains("\n  def test_é(:\n             ^"),
                "{message}"
            );
            assert!(!message.contains("byte offset"));
        }
    }
}

#[test]
fn source_carets_respect_wide_characters_combining_marks_and_tabs() {
    let project = TempDir::new().unwrap();
    for (source, location, evidence) in [
        (
            "def test_雪(:\n",
            "test_unicode.py:1:12:",
            "\n  def test_雪(:\n              ^",
        ),
        (
            "def test_e\u{301}(:\n",
            "test_unicode.py:1:13:",
            "\n  def test_e\u{301}(:\n             ^",
        ),
        (
            "def test_ok():\n\t雪 = :\n",
            "test_unicode.py:2:6:",
            "\n  \t雪 = :\n  \t     ^",
        ),
    ] {
        fs::write(project.path().join("test_unicode.py"), source).unwrap();
        let output = run(&project, &["list", "test_unicode.py", "--json"]);
        assert_eq!(output.status.code(), Some(2));
        let report = json(&output);
        let message = report["error"].as_str().unwrap();
        assert!(message.contains(location), "{message}");
        assert!(message.contains(evidence), "{message}");
    }
}
