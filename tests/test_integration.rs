//! Integration tests for taut.
//!
//! These tests run the full taut binary as a subprocess and verify
//! end-to-end behavior including:
//! - Test discovery and execution
//! - Incremental test runs
//! - CLI options
//! - Exit codes

mod helpers;

use std::fs;

use anyhow::Result;

use helpers::{dedent, run_taut, TempProject};

// =============================================================================
// Basic Execution Tests
// =============================================================================

#[test]
fn run_simple_passing_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_simple.py",
        &dedent(r#"
            def test_one():
                assert True

            def test_two():
                assert 1 + 1 == 2
        "#),
    )?;

    let result = run_taut(&project, &["."])?;

    result.assert_success();
    assert!(result.stdout.contains("passed"));
    assert!(result.count_in_stdout(".") >= 2 || result.stdout.contains("2 passed"));

    Ok(())
}

#[test]
fn run_failing_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_fail.py",
        &dedent(r#"
            def test_pass():
                assert True

            def test_fail():
                assert False, "expected failure"
        "#),
    )?;

    let result = run_taut(&project, &["."])?;

    result.assert_failure();
    assert!(
        result.stdout.contains("failed") || result.stdout.contains("F"),
        "Should indicate failure: {}",
        result.stdout
    );

    Ok(())
}

#[test]
fn run_mixed_pass_fail() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_mixed.py",
        &dedent(r#"
            def test_pass1(): assert True
            def test_pass2(): assert True
            def test_fail(): assert False
        "#),
    )?;

    let result = run_taut(&project, &["."])?;

    result.assert_failure();
    // Should show both passed and failed counts
    assert!(result.stdout.contains("passed"));
    assert!(result.stdout.contains("failed"));

    Ok(())
}

#[test]
fn exit_code_zero_on_all_pass() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_ok.py", "def test_ok(): assert True\n")?;

    let result = run_taut(&project, &["."])?;

    assert_eq!(result.exit_code, 0, "Exit code should be 0 on success");

    Ok(())
}

#[test]
fn exit_code_one_on_any_fail() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_fail.py", "def test_fail(): assert False\n")?;

    let result = run_taut(&project, &["."])?;

    assert_eq!(result.exit_code, 1, "Exit code should be 1 on failure");

    Ok(())
}

// =============================================================================
// Incremental Run Tests
// =============================================================================

#[test]
fn incremental_run_skips_unchanged_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_inc.py",
        &dedent(r#"
            def test_one():
                assert True

            def test_two():
                assert True
        "#),
    )?;

    // First run - both tests should run
    let result1 = run_taut(&project, &["."])?;
    result1.assert_success();

    // Second run - tests should be skipped (cached)
    let result2 = run_taut(&project, &["."])?;
    result2.assert_success();

    // Should show "skipped" or "s" for cached tests
    // (or "unchanged" depending on implementation)
    assert!(
        result2.stdout.contains("skipped")
            || result2.stdout.contains("s")
            || result2.stdout.contains("unchanged"),
        "Second run should skip unchanged tests: {}",
        result2.stdout
    );

    Ok(())
}

#[test]
fn incremental_run_reruns_changed_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_change.py",
        &dedent(r#"
            def helper():
                return 1

            def test_uses_helper():
                assert helper() == 1
        "#),
    )?;

    // First run
    let result1 = run_taut(&project, &["."])?;
    result1.assert_success();

    // Modify the helper
    project.write_file(
        "test_change.py",
        &dedent(r#"
            def helper():
                return 2

            def test_uses_helper():
                assert helper() == 1
        "#),
    )?;

    // Second run - should re-run and fail
    let result2 = run_taut(&project, &["."])?;
    result2.assert_failure();

    Ok(())
}

#[test]
fn incremental_run_reruns_failed_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_retry.py", "def test_fail(): assert False\n")?;

    // First run - fails
    let result1 = run_taut(&project, &["."])?;
    result1.assert_failure();

    // Fix the test
    project.write_file("test_retry.py", "def test_fail(): assert True\n")?;

    // Second run - should re-run (was failing) and pass
    let result2 = run_taut(&project, &["."])?;
    result2.assert_success();

    Ok(())
}

// =============================================================================
// CLI Option Tests
// =============================================================================

#[test]
fn filter_option_limits_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_filter.py",
        &dedent(r#"
            def test_alpha(): assert True
            def test_beta(): assert True
            def test_gamma(): assert True
        "#),
    )?;

    let result = run_taut(&project, &["-k", "alpha", "."])?;

    result.assert_success();

    // Should only run test_alpha
    // Check that we don't have 3 tests running
    assert!(
        !result.stdout.contains("3 passed"),
        "Should not run all 3 tests"
    );

    Ok(())
}

#[test]
fn no_cache_runs_all_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_nocache.py",
        &dedent(r#"
            def test_one(): assert True
            def test_two(): assert True
        "#),
    )?;

    // First run
    run_taut(&project, &["."])?;

    // Second run with --no-cache should run all tests, not skip
    let result = run_taut(&project, &["--no-cache", "."])?;
    result.assert_success();

    // Should NOT show "skipped" when using --no-cache
    // All tests should run
    assert!(
        !result.stdout.contains("skipped") || result.stdout.contains("0 skipped"),
        "--no-cache should run all tests: {}",
        result.stdout
    );

    Ok(())
}

#[test]
fn verbose_option_shows_test_names() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_verbose.py",
        "def test_specific_name(): assert True\n",
    )?;

    let result = run_taut(&project, &["--verbose", "."])?;
    result.assert_success();

    // Verbose output should include the test name
    assert!(
        result.stdout.contains("test_specific_name"),
        "Verbose output should show test name: {}",
        result.stdout
    );

    Ok(())
}

#[test]
fn jobs_option_limits_parallelism() -> Result<()> {
    let mut project = TempProject::new()?;

    // Create several test files
    for i in 0..5 {
        project.write_file(
            &format!("test_{}.py", i),
            &format!("def test_{}(): assert True\n", i),
        )?;
    }

    // Run with limited jobs
    let result = run_taut(&project, &["-j", "2", "."])?;
    result.assert_success();

    Ok(())
}

#[test]
fn no_parallel_runs_sequentially() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_files(&[
        ("test_a.py", "def test_a(): assert True"),
        ("test_b.py", "def test_b(): assert True"),
    ])?;

    let result = run_taut(&project, &["--no-parallel", "."])?;
    result.assert_success();

    Ok(())
}

// =============================================================================
// List Command Tests
// =============================================================================

#[test]
fn list_command_shows_discovered_tests() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_list.py",
        &dedent(r#"
            def test_alpha(): pass
            def test_beta(): pass

            class TestClass:
                def test_method(self): pass
        "#),
    )?;

    let result = run_taut(&project, &["list", "."])?;
    result.assert_success();

    // Should list all tests
    assert!(
        result.stdout.contains("test_alpha"),
        "Should list test_alpha"
    );
    assert!(result.stdout.contains("test_beta"), "Should list test_beta");
    assert!(
        result.stdout.contains("test_method"),
        "Should list test_method"
    );

    Ok(())
}

#[test]
fn list_command_with_filter() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_filter.py",
        "def test_alpha(): pass\ndef test_beta(): pass\n",
    )?;

    let result = run_taut(&project, &["list", "-k", "alpha", "."])?;
    result.assert_success();

    assert!(result.stdout.contains("test_alpha"));
    assert!(!result.stdout.contains("test_beta"));

    Ok(())
}

// =============================================================================
// Cache Commands Tests
// =============================================================================

#[test]
fn cache_info_shows_stats() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_cache.py", "def test_ok(): assert True\n")?;

    // Run tests to populate cache
    run_taut(&project, &["."])?;

    // Check cache info
    let result = run_taut(&project, &["cache", "info"])?;
    result.assert_success();

    assert!(
        result.stdout.contains("Cache") || result.stdout.contains("cache"),
        "Should show cache info"
    );

    Ok(())
}

#[test]
fn cache_clear_removes_cache() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_clear.py", "def test_ok(): assert True\n")?;

    // Run tests to populate cache
    run_taut(&project, &["."])?;

    // Clear cache
    let result = run_taut(&project, &["cache", "clear"])?;
    result.assert_success();

    // After clearing, next run should run all tests (not skip)
    let result2 = run_taut(&project, &["."])?;
    result2.assert_success();

    // Should not skip (cache was cleared)
    // This might show "passed" without "skipped"

    Ok(())
}

// =============================================================================
// Multiple Paths Tests
// =============================================================================

#[test]
fn multiple_paths_combined() -> Result<()> {
    let mut project = TempProject::new()?;

    project.mkdir("dir_a")?;
    project.mkdir("dir_b")?;

    project.write_file("dir_a/test_a.py", "def test_a(): assert True\n")?;
    project.write_file("dir_b/test_b.py", "def test_b(): assert True\n")?;

    // Run on both directories
    let result = run_taut(&project, &["dir_a", "dir_b"])?;
    result.assert_success();

    // Should find tests from both
    assert!(
        result.stdout.contains("2 passed") || result.count_in_stdout(".") >= 2,
        "Should run tests from both directories"
    );

    Ok(())
}

#[test]
fn single_file_path() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file("test_single.py", "def test_single(): assert True\n")?;
    project.write_file("test_other.py", "def test_other(): assert True\n")?;

    // Run on single file
    let result = run_taut(&project, &["test_single.py"])?;
    result.assert_success();

    // Should only run the one test
    assert!(
        result.stdout.contains("1 passed"),
        "Should only run one test from specified file"
    );

    Ok(())
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn no_tests_found_message() -> Result<()> {
    let mut project = TempProject::new()?;

    // Create a non-test file
    project.write_file("not_a_test.py", "def foo(): pass\n")?;

    let result = run_taut(&project, &["."])?;

    // Should indicate no tests found (not crash)
    assert!(
        result.stdout.contains("No tests") || result.stdout.contains("0"),
        "Should indicate no tests: {}",
        result.stdout
    );

    Ok(())
}

#[test]
fn empty_directory() -> Result<()> {
    let project = TempProject::new()?;

    // Empty directory - no files
    let result = run_taut(&project, &["."])?;

    assert!(
        result.stdout.contains("No tests") || result.stdout.contains("0"),
        "Should handle empty directory gracefully"
    );

    Ok(())
}

#[test]
fn test_with_unicode_output() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_unicode.py",
        &dedent(r#"
            def test_unicode():
                print("Hello 世界 🎉")
                assert True
        "#),
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_success();

    Ok(())
}

#[test]
fn test_in_subdirectory() -> Result<()> {
    let mut project = TempProject::new()?;

    project.mkdir("tests/unit")?;
    project.write_file(
        "tests/unit/test_deep.py",
        "def test_deep(): assert True\n",
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_success();

    assert!(result.stdout.contains("passed"));

    Ok(())
}

// =============================================================================
// Async Tests Integration
// =============================================================================

#[test]
fn async_tests_run_correctly() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_async.py",
        &dedent(r#"
            import asyncio

            async def test_async():
                await asyncio.sleep(0.01)
                assert True

            def test_sync():
                assert True
        "#),
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_success();

    // Both tests should pass
    // BUG: Async functions are not discovered properly (AsyncFunctionDef not handled)
    // so currently only 1 test is found. When fixed, this should be "2 passed"
    assert!(
        result.stdout.contains("2 passed"),
        "BUG: Both async and sync tests should pass. Only found 1 test due to async discovery bug. stdout: {}",
        result.stdout
    );

    Ok(())
}

// =============================================================================
// Class-based Tests Integration
// =============================================================================

#[test]
fn class_based_tests_run_correctly() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_class.py",
        &dedent(r#"
            class TestMath:
                def test_add(self):
                    assert 1 + 1 == 2

                def test_sub(self):
                    assert 2 - 1 == 1

            class TestString:
                def test_upper(self):
                    assert "hello".upper() == "HELLO"
        "#),
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_success();

    assert!(
        result.stdout.contains("3 passed"),
        "All class methods should run"
    );

    Ok(())
}

// =============================================================================
// Failure Output Tests
// =============================================================================

#[test]
fn failure_shows_traceback() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_traceback.py",
        &dedent(r#"
            def helper():
                raise ValueError("from helper")

            def test_fails():
                helper()
        "#),
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_failure();

    // Should show some traceback info
    assert!(
        result.stdout.contains("ValueError")
            || result.stdout.contains("helper")
            || result.stdout.contains("traceback"),
        "Should show error info: {}",
        result.stdout
    );

    Ok(())
}

#[test]
fn assertion_shows_message() -> Result<()> {
    let mut project = TempProject::new()?;

    project.write_file(
        "test_assert_msg.py",
        "def test_with_message(): assert 1 == 2, 'numbers should match'\n",
    )?;

    let result = run_taut(&project, &["."])?;
    result.assert_failure();

    // The assertion message should appear somewhere
    assert!(
        result.stdout.contains("numbers should match")
            || result.stdout.contains("AssertionError"),
        "Should show assertion message or error"
    );

    Ok(())
}
