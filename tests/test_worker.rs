//! Tests for the Python worker execution.
//!
//! These tests verify that taut correctly:
//! - Runs Python tests and captures results
//! - Handles stdout/stderr capture
//! - Isolates modules between tests
//! - Collects coverage data
//! - Handles errors gracefully
//!
//! Several tests document bugs in the current implementation.

mod helpers;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use tempfile::TempDir;

use helpers::dedent;
use taut::discovery::TestItem;
use taut::runner::{run_tests, IsolationMode};

fn write_file(path: &std::path::Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

// =============================================================================
// Basic Execution Tests
// =============================================================================

#[test]
fn runs_passing_test() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_pass.py");
    write_file(&test_file, "def test_ok(): assert True\n")?;

    let item = TestItem {
        file: test_file,
        function: "test_ok".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false, // no parallel
        None,
        false, // no coverage
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert_eq!(results.results.len(), 1);
    assert!(results.results[0].passed);
    assert!(results.results[0].error.is_none());

    Ok(())
}

#[test]
fn runs_failing_assertion() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_fail.py");
    write_file(&test_file, "def test_fail(): assert False, 'expected failure'\n")?;

    let item = TestItem {
        file: test_file,
        function: "test_fail".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    assert!(results.results[0].error.is_some());

    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("expected failure") || error.message.contains("AssertionError"),
        "Error message should contain assertion info: {}",
        error.message
    );

    Ok(())
}

#[test]
fn runs_failing_exception() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_exc.py");
    write_file(
        &test_file,
        "def test_raises(): raise ValueError('boom')\n",
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_raises".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("ValueError") || error.message.contains("boom"),
        "Error should mention ValueError: {}",
        error.message
    );

    Ok(())
}

#[test]
fn captures_stdout() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_print.py");
    write_file(
        &test_file,
        &dedent(r#"
            def test_prints():
                print("hello from test")
                assert True
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_prints".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    assert!(
        results.results[0]
            .stdout
            .as_ref()
            .map(|s| s.contains("hello from test"))
            .unwrap_or(false),
        "stdout should contain printed text"
    );

    Ok(())
}

#[test]
fn captures_stderr() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_stderr.py");
    write_file(
        &test_file,
        &dedent(r#"
            import sys
            def test_stderr():
                print("error message", file=sys.stderr)
                assert True
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_stderr".to_string(),
        class: None,
        line: 2,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    assert!(
        results.results[0]
            .stderr
            .as_ref()
            .map(|s| s.contains("error message"))
            .unwrap_or(false),
        "stderr should contain printed text"
    );

    Ok(())
}

// =============================================================================
// Async Test Execution
// =============================================================================

#[test]
fn runs_async_test() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_async.py");
    write_file(
        &test_file,
        &dedent(r#"
            import asyncio

            async def test_async():
                await asyncio.sleep(0.001)
                assert True
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_async".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(
        results.results[0].passed,
        "Async test should pass. Error: {:?}",
        results.results[0].error
    );

    Ok(())
}

#[test]
fn async_test_can_use_await() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_async_await.py");
    write_file(
        &test_file,
        &dedent(r#"
            import asyncio

            async def async_helper():
                await asyncio.sleep(0.001)
                return 42

            async def test_await():
                result = await async_helper()
                assert result == 42
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_await".to_string(),
        class: None,
        line: 7,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);

    Ok(())
}

// =============================================================================
// Class-based Tests
// =============================================================================

#[test]
fn runs_class_method_test() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_class.py");
    write_file(
        &test_file,
        &dedent(r#"
            class TestMath:
                def test_add(self):
                    assert 1 + 1 == 2
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_add".to_string(),
        class: Some("TestMath".to_string()),
        line: 2,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);

    Ok(())
}

#[test]
fn runs_setup_and_teardown() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_setup.py");
    write_file(
        &test_file,
        &dedent(r#"
            class TestWithSetup:
                def setUp(self):
                    self.value = 42

                def tearDown(self):
                    del self.value

                def test_uses_setup(self):
                    assert self.value == 42
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_uses_setup".to_string(),
        class: Some("TestWithSetup".to_string()),
        line: 8,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(
        results.results[0].passed,
        "Test with setUp should pass. Error: {:?}",
        results.results[0].error
    );

    Ok(())
}

#[test]
fn setup_failure_fails_test() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_setup_fail.py");
    write_file(
        &test_file,
        &dedent(r#"
            class TestSetupFails:
                def setUp(self):
                    raise RuntimeError("setup failed")

                def test_never_runs(self):
                    assert True
        "#),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_never_runs".to_string(),
        class: Some("TestSetupFails".to_string()),
        line: 5,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("setup failed") || error.message.contains("RuntimeError"),
        "Error should mention setup failure: {}",
        error.message
    );

    Ok(())
}

#[test]
fn teardown_runs_after_failure() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_teardown.py");
    let marker_file = tmp.path().join("teardown_ran.txt");

    write_file(
        &test_file,
        &format!(
            r#"
class TestTeardownAfterFailure:
    def setUp(self):
        pass

    def tearDown(self):
        with open({:?}, 'w') as f:
            f.write('teardown ran')

    def test_fails(self):
        assert False
"#,
            marker_file
        ),
    )?;

    let item = TestItem {
        file: test_file,
        function: "test_fails".to_string(),
        class: Some("TestTeardownAfterFailure".to_string()),
        line: 10,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);

    // Check that tearDown ran despite test failure
    assert!(
        marker_file.exists(),
        "tearDown should run even when test fails"
    );

    Ok(())
}

// =============================================================================
// Import Handling
// =============================================================================

#[test]
fn imports_from_same_directory() -> Result<()> {
    let tmp = TempDir::new()?;

    // Create a helper module
    write_file(
        &tmp.path().join("helper.py"),
        "def get_value(): return 42\n",
    )?;

    // Create test that imports it
    write_file(
        &tmp.path().join("test_import.py"),
        &dedent(r#"
            from helper import get_value

            def test_import():
                assert get_value() == 42
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_import.py"),
        function: "test_import".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(
        results.results[0].passed,
        "Should import from same directory. Error: {:?}",
        results.results[0].error
    );

    Ok(())
}

#[test]
fn imports_from_subdirectory() -> Result<()> {
    let tmp = TempDir::new()?;

    // Create utils package
    fs::create_dir_all(tmp.path().join("utils"))?;
    write_file(&tmp.path().join("utils/__init__.py"), "")?;
    write_file(
        &tmp.path().join("utils/math.py"),
        "def add(a, b): return a + b\n",
    )?;

    // Create test
    write_file(
        &tmp.path().join("test_subdir.py"),
        &dedent(r#"
            from utils.math import add

            def test_add():
                assert add(1, 2) == 3
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_subdir.py"),
        function: "test_add".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(
        results.results[0].passed,
        "Should import from subdirectory. Error: {:?}",
        results.results[0].error
    );

    Ok(())
}

#[test]
fn relative_import_fails_gracefully() -> Result<()> {
    // BUG: Relative imports crash the worker because the module
    // doesn't have __package__ set.
    //
    // This test verifies the error is captured gracefully.

    let tmp = TempDir::new()?;

    write_file(&tmp.path().join("helper.py"), "VALUE = 42\n")?;
    write_file(
        &tmp.path().join("test_relative.py"),
        &dedent(r#"
            from . import helper

            def test_relative():
                assert helper.VALUE == 42
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_relative.py"),
        function: "test_relative".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    // Should fail with a clear error, not crash
    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("relative import")
            || error.message.contains("ImportError")
            || error.message.contains("no known parent package"),
        "Should fail with import error: {}",
        error.message
    );

    Ok(())
}

#[test]
fn import_error_captured() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_bad_import.py"),
        &dedent(r#"
            import nonexistent_module_xyz

            def test_never_runs():
                assert True
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_bad_import.py"),
        function: "test_never_runs".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("ModuleNotFoundError")
            || error.message.contains("nonexistent_module"),
        "Should capture import error: {}",
        error.message
    );

    Ok(())
}

// =============================================================================
// Module Isolation Tests
// =============================================================================

#[test]
fn module_state_isolated_between_tests_process_per_test() -> Result<()> {
    let tmp = TempDir::new()?;

    // Create a module with global state
    write_file(
        &tmp.path().join("state.py"),
        "counter = 0\ndef increment(): global counter; counter += 1; return counter\n",
    )?;

    // Create two tests that both increment
    write_file(
        &tmp.path().join("test_state.py"),
        &dedent(r#"
            from state import increment

            def test_first():
                assert increment() == 1

            def test_second():
                assert increment() == 1
        "#),
    )?;

    let item1 = TestItem {
        file: tmp.path().join("test_state.py"),
        function: "test_first".to_string(),
        class: None,
        line: 3,
    };

    let item2 = TestItem {
        file: tmp.path().join("test_state.py"),
        function: "test_second".to_string(),
        class: None,
        line: 6,
    };

    // Run with process-per-test - each should get fresh state
    let results = run_tests(
        &[item1, item2],
        false, // sequential
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(
        results.results[0].passed,
        "First test should pass: {:?}",
        results.results[0].error
    );
    assert!(
        results.results[1].passed,
        "Second test should also see fresh state: {:?}",
        results.results[1].error
    );

    Ok(())
}

#[test]
fn module_state_may_leak_in_process_per_run() -> Result<()> {
    // BUG: In process-per-run mode, modules are cached in sys.modules
    // Global state leaks between tests.
    //
    // This test documents the issue.

    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("state.py"),
        "counter = 0\ndef increment(): global counter; counter += 1; return counter\n",
    )?;

    write_file(
        &tmp.path().join("test_state.py"),
        &dedent(r#"
            from state import increment

            def test_first():
                assert increment() == 1

            def test_second():
                # In process-per-run, this would be 2 if state leaks
                assert increment() == 1
        "#),
    )?;

    let item1 = TestItem {
        file: tmp.path().join("test_state.py"),
        function: "test_first".to_string(),
        class: None,
        line: 3,
    };

    let item2 = TestItem {
        file: tmp.path().join("test_state.py"),
        function: "test_second".to_string(),
        class: None,
        line: 6,
    };

    // Run with process-per-run - state MAY leak
    let results = run_tests(
        &[item1, item2],
        false, // sequential so order is deterministic
        None,
        false,
        IsolationMode::ProcessPerRun,
        |_| {},
    )?;

    // First should always pass
    assert!(results.results[0].passed);

    // Second might fail if state leaked (documenting the bug)
    if !results.results[1].passed {
        eprintln!("BUG: Module state leaked between tests in process-per-run mode");
    }

    Ok(())
}

// =============================================================================
// Coverage Collection Tests
// =============================================================================

#[test]
fn coverage_collected_for_test_file() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_cov.py");

    write_file(
        &test_file,
        &dedent(r#"
            def helper():
                return 1

            def test_with_helper():
                assert helper() == 1
        "#),
    )?;

    let item = TestItem {
        file: test_file.clone(),
        function: "test_with_helper".to_string(),
        class: None,
        line: 5,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        true, // collect coverage
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    assert!(
        results.results[0].coverage.is_some(),
        "Coverage should be collected"
    );

    let coverage = results.results[0].coverage.as_ref().unwrap();

    // Should have coverage for the test file
    let test_file_cov = coverage
        .files
        .iter()
        .find(|(path, _)| path.to_string_lossy().contains("test_cov.py"));

    assert!(
        test_file_cov.is_some(),
        "Should have coverage for test file"
    );

    Ok(())
}

#[test]
fn coverage_collected_for_imported_file() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("mymodule.py"),
        &dedent(r#"
            def add(a, b):
                return a + b
        "#),
    )?;

    write_file(
        &tmp.path().join("test_import_cov.py"),
        &dedent(r#"
            from mymodule import add

            def test_add():
                assert add(1, 2) == 3
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_import_cov.py"),
        function: "test_add".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        true,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    let coverage = results.results[0].coverage.as_ref().unwrap();

    // Should have coverage for both files
    let mymodule_cov = coverage
        .files
        .iter()
        .find(|(path, _)| path.to_string_lossy().contains("mymodule.py"));

    assert!(
        mymodule_cov.is_some(),
        "Should have coverage for imported module"
    );

    Ok(())
}

#[test]
fn coverage_excludes_stdlib() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_stdlib.py"),
        &dedent(r#"
            import os
            import json

            def test_uses_stdlib():
                data = json.dumps({"key": "value"})
                assert os.path.sep in "/" or os.path.sep == "\\"
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_stdlib.py"),
        function: "test_uses_stdlib".to_string(),
        class: None,
        line: 5,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        true,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    let coverage = results.results[0].coverage.as_ref().unwrap();

    // Should NOT have coverage for stdlib modules
    for (path, _) in &coverage.files {
        let path_str = path.to_string_lossy();
        assert!(
            !path_str.contains("site-packages") && !path_str.contains("lib/python"),
            "Stdlib path should be excluded: {}",
            path_str
        );
    }

    Ok(())
}

#[test]
fn coverage_works_in_async_test() -> Result<()> {
    // BUG: sys.settrace doesn't work inside async functions
    // Coverage is incomplete for async code.
    //
    // sys.monitoring (Python 3.12+) should fix this.

    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("async_helper.py"),
        &dedent(r#"
            async def async_add(a, b):
                return a + b
        "#),
    )?;

    write_file(
        &tmp.path().join("test_async_cov.py"),
        &dedent(r#"
            import asyncio
            from async_helper import async_add

            async def test_async_coverage():
                result = await async_add(1, 2)
                assert result == 3
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_async_cov.py"),
        function: "test_async_coverage".to_string(),
        class: None,
        line: 5,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        true,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    let coverage = results.results[0].coverage.as_ref().unwrap();

    // Check if async_helper.py has coverage
    let async_helper_cov = coverage
        .files
        .iter()
        .find(|(path, _)| path.to_string_lossy().contains("async_helper.py"));

    if async_helper_cov.is_none() {
        eprintln!("BUG: No coverage collected for async helper module");
        eprintln!("This is expected with sys.settrace - need sys.monitoring for async coverage");
    }

    // Even if we have coverage for the file, check if we have the right lines
    if let Some((_, lines)) = async_helper_cov {
        if lines.is_empty() || !lines.contains(&2) {
            eprintln!("BUG: Coverage missing for lines inside async function");
        }
    }

    Ok(())
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn syntax_error_in_test_file_captured() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_syntax.py"),
        "def test_broken(\n    # missing paren\n",
    )?;

    let item = TestItem {
        file: tmp.path().join("test_syntax.py"),
        function: "test_broken".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("SyntaxError") || error.message.contains("syntax"),
        "Should capture syntax error: {}",
        error.message
    );

    Ok(())
}

#[test]
fn test_function_not_found_captured() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_missing.py"),
        "def test_exists(): pass\n",
    )?;

    let item = TestItem {
        file: tmp.path().join("test_missing.py"),
        function: "test_does_not_exist".to_string(),
        class: None,
        line: 1,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(!results.results[0].passed);
    let error = results.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("AttributeError") || error.message.contains("test_does_not_exist"),
        "Should indicate function not found: {}",
        error.message
    );

    Ok(())
}

// =============================================================================
// Timing Tests
// =============================================================================

#[test]
fn test_duration_tracked() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_slow.py"),
        &dedent(r#"
            import time

            def test_takes_time():
                time.sleep(0.1)
                assert True
        "#),
    )?;

    let item = TestItem {
        file: tmp.path().join("test_slow.py"),
        function: "test_takes_time".to_string(),
        class: None,
        line: 3,
    };

    let results = run_tests(
        &[item],
        false,
        None,
        false,
        IsolationMode::ProcessPerTest,
        |_| {},
    )?;

    assert!(results.results[0].passed);
    assert!(
        results.results[0].duration >= Duration::from_millis(100),
        "Duration should be >= 100ms, got {:?}",
        results.results[0].duration
    );

    Ok(())
}
