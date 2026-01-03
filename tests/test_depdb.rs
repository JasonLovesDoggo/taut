//! Tests for the dependency database and test selection.
//!
//! These tests verify that taut correctly:
//! - Tracks test dependencies on code blocks
//! - Decides which tests need to re-run based on changes
//! - Handles cache persistence and invalidation
//!
//! Several tests document bugs that need fixing.

mod helpers;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use tempfile::TempDir;

use helpers::dedent;
use taut::blocks::FileBlocks;
use taut::depdb::{DependencyDatabase, TestRunDecision};
use taut::discovery::TestItem;

// =============================================================================
// Test Selection - Basic Cases
// =============================================================================

#[test]
fn new_test_always_runs() {
    let depdb = DependencyDatabase::default();

    let test = TestItem {
        file: PathBuf::from("test_foo.py"),
        function: "test_new".to_string(),
        class: None,
        line: 1,
    };

    let decision = depdb.needs_run(&test);

    assert!(
        matches!(decision, TestRunDecision::NeverRun),
        "New test should have NeverRun decision"
    );
    assert!(decision.should_run());
}

#[test]
fn failed_test_always_reruns() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    fs::write(&test_file, "def test_fail(): assert False\n")?;

    let mut depdb = DependencyDatabase::default();
    let block_index = HashMap::new();

    let test = TestItem {
        file: test_file.clone(),
        function: "test_fail".to_string(),
        class: None,
        line: 1,
    };

    // Record that the test failed
    depdb.record_test_coverage(&test, &HashMap::new(), false, &block_index);

    let decision = depdb.needs_run(&test);

    assert!(
        matches!(decision, TestRunDecision::FailedLastTime),
        "Failed test should have FailedLastTime decision, got {:?}",
        decision
    );
    assert!(decision.should_run());

    Ok(())
}

#[test]
fn unchanged_passing_test_skips() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    fs::write(&test_file, "def test_pass(): assert True\n")?;

    let mut depdb = DependencyDatabase::default();

    // Parse the file to get blocks
    let file_blocks = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_pass".to_string(),
        class: None,
        line: 1,
    };

    // Record that the test passed with some coverage
    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Now check if it needs to run again (it shouldn't - nothing changed)
    let decision = depdb.needs_run(&test);

    assert!(
        matches!(decision, TestRunDecision::CanSkip),
        "Unchanged passing test should skip, got {:?}",
        decision
    );
    assert!(!decision.should_run());

    Ok(())
}

#[test]
fn changed_dependency_reruns() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    let code_v1 = &dedent(
        r#"
        def helper():
            return 1

        def test_uses_helper():
            assert helper() == 1
    "#,
    );
    fs::write(&test_file, code_v1)?;

    let mut depdb = DependencyDatabase::default();

    // First run: parse, record coverage
    let file_blocks_v1 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v1);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks_v1);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_uses_helper".to_string(),
        class: None,
        line: 5,
    };

    // Record coverage: test touched lines 1-2 (helper) and 5-6 (test)
    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1, 2, 5, 6]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Now change the helper function
    let code_v2 = &dedent(
        r#"
        def helper():
            return 2

        def test_uses_helper():
            assert helper() == 1
    "#,
    );
    fs::write(&test_file, code_v2)?;

    // Re-parse with new content
    let file_blocks_v2 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v2);

    // Check if test needs to run
    let decision = depdb.needs_run(&test);

    assert!(
        matches!(decision, TestRunDecision::DependencyChanged),
        "Test should re-run when dependency changed, got {:?}",
        decision
    );

    Ok(())
}

// =============================================================================
// BUG: Line Number Fragility
// =============================================================================

#[test]
fn adding_blank_line_should_not_invalidate_cache() -> Result<()> {
    // CRITICAL BUG: Currently, BlockId includes start_line and end_line.
    // When a blank line is added above a function, the line numbers change,
    // which changes the BlockId key, which causes DependencyDeleted.
    //
    // This test will FAIL until fixed.

    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");

    // Version 1: no blank line
    let code_v1 = &dedent(
        r#"
        def helper():
            return 1

        def test_foo():
            assert helper() == 1
    "#,
    );
    fs::write(&test_file, code_v1)?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks_v1 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v1);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks_v1);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_foo".to_string(),
        class: None,
        line: 4,
    };

    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1, 2, 4, 5]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Verify test can skip now
    assert!(
        matches!(depdb.needs_run(&test), TestRunDecision::CanSkip),
        "Test should skip initially"
    );

    // Version 2: add blank line above helper (NO CODE CHANGE)
    let code_v2 = &dedent(
        r#"

        def helper():
            return 1

        def test_foo():
            assert helper() == 1
    "#,
    );
    fs::write(&test_file, code_v2)?;

    let file_blocks_v2 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v2);

    // Update test's line number since it moved
    let test_v2 = TestItem {
        file: test_file.canonicalize()?,
        function: "test_foo".to_string(),
        class: None,
        line: 5, // Line number changed
    };

    let decision = depdb.needs_run(&test_v2);

    // BUG: Currently returns DependencyDeleted because the BlockId key changed
    // SHOULD return CanSkip because no actual code changed
    assert!(
        matches!(decision, TestRunDecision::CanSkip),
        "BUG: Adding blank line should NOT invalidate cache. Got {:?}",
        decision
    );

    Ok(())
}

#[test]
fn adding_comment_should_not_invalidate_cache() -> Result<()> {
    // Similar bug: adding a comment changes line numbers

    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");

    let code_v1 = "def helper(): return 1\ndef test_foo(): assert helper() == 1\n";
    fs::write(&test_file, code_v1)?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks_v1 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v1);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks_v1);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_foo".to_string(),
        class: None,
        line: 2,
    };

    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1, 2]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Add a comment at the top
    let code_v2 = "# New comment\ndef helper(): return 1\ndef test_foo(): assert helper() == 1\n";
    fs::write(&test_file, code_v2)?;

    let file_blocks_v2 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v2);

    let test_v2 = TestItem {
        file: test_file.canonicalize()?,
        function: "test_foo".to_string(),
        class: None,
        line: 3,
    };

    let decision = depdb.needs_run(&test_v2);

    assert!(
        matches!(decision, TestRunDecision::CanSkip),
        "BUG: Adding comment should NOT invalidate cache. Got {:?}",
        decision
    );

    Ok(())
}

#[test]
fn reordering_functions_with_same_content_should_not_invalidate() -> Result<()> {
    // If we move a function but don't change its content,
    // tests depending on it should not re-run.

    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");

    let code_v1 = &dedent(
        r#"
        def helper_a():
            return 1

        def helper_b():
            return 2

        def test_uses_a():
            assert helper_a() == 1
    "#,
    );
    fs::write(&test_file, code_v1)?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks_v1 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v1);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks_v1);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_uses_a".to_string(),
        class: None,
        line: 8,
    };

    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1, 2, 8, 9]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Reorder: swap helper_a and helper_b
    let code_v2 = &dedent(
        r#"
        def helper_b():
            return 2

        def helper_a():
            return 1

        def test_uses_a():
            assert helper_a() == 1
    "#,
    );
    fs::write(&test_file, code_v2)?;

    let file_blocks_v2 = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks_v2);

    let test_v2 = TestItem {
        file: test_file.canonicalize()?,
        function: "test_uses_a".to_string(),
        class: None,
        line: 8,
    };

    let decision = depdb.needs_run(&test_v2);

    // The content of helper_a didn't change, just its position
    // Ideally this should skip, but with line-based BlockId it won't
    assert!(
        matches!(decision, TestRunDecision::CanSkip),
        "BUG: Reordering functions should NOT invalidate cache if content unchanged. Got {:?}",
        decision
    );

    Ok(())
}

// =============================================================================
// Path Handling
// =============================================================================

#[test]
fn relative_and_absolute_paths_should_match() -> Result<()> {
    // BUG: Discovery may use relative paths while selection uses absolute paths
    // This can cause tests to always appear as "new"

    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    fs::write(&test_file, "def test_ok(): pass\n")?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks);

    // Record with absolute path
    let test_abs = TestItem {
        file: test_file.canonicalize()?,
        function: "test_ok".to_string(),
        class: None,
        line: 1,
    };

    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![1]);
    depdb.record_test_coverage(&test_abs, &coverage, true, &block_index);

    // Query with relative path
    let test_rel = TestItem {
        file: test_file.clone(), // Not canonicalized
        function: "test_ok".to_string(),
        class: None,
        line: 1,
    };

    let decision_abs = depdb.needs_run(&test_abs);
    let decision_rel = depdb.needs_run(&test_rel);

    // Both should give the same result
    // BUG: Currently they might differ
    assert_eq!(
        decision_abs.should_run(),
        decision_rel.should_run(),
        "Relative and absolute paths should give same result"
    );

    Ok(())
}

#[test]
fn different_files_same_function_name() -> Result<()> {
    let tmp = TempDir::new()?;

    let file_a = tmp.path().join("test_a.py");
    let file_b = tmp.path().join("test_b.py");

    fs::write(&file_a, "def test_common(): pass\n")?;
    fs::write(&file_b, "def test_common(): pass\n")?;

    let mut depdb = DependencyDatabase::default();

    let blocks_a = FileBlocks::from_file(&file_a)?;
    let blocks_b = FileBlocks::from_file(&file_b)?;
    depdb.update_blocks(&blocks_a);
    depdb.update_blocks(&blocks_b);

    let mut block_index = HashMap::new();
    block_index.insert(file_a.canonicalize()?, blocks_a);
    block_index.insert(file_b.canonicalize()?, blocks_b);

    let test_a = TestItem {
        file: file_a.canonicalize()?,
        function: "test_common".to_string(),
        class: None,
        line: 1,
    };

    let test_b = TestItem {
        file: file_b.canonicalize()?,
        function: "test_common".to_string(),
        class: None,
        line: 1,
    };

    // Record coverage for test_a only
    let mut coverage = HashMap::new();
    coverage.insert(file_a.canonicalize()?, vec![1]);
    depdb.record_test_coverage(&test_a, &coverage, true, &block_index);

    // test_a should skip, test_b should run (never recorded)
    let decision_a = depdb.needs_run(&test_a);
    let decision_b = depdb.needs_run(&test_b);

    assert!(
        matches!(decision_a, TestRunDecision::CanSkip),
        "test_a should skip"
    );
    assert!(
        matches!(decision_b, TestRunDecision::NeverRun),
        "test_b should be NeverRun"
    );

    Ok(())
}

#[test]
fn same_method_name_different_classes() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_classes.py");

    let code = &dedent(
        r#"
        class TestAlpha:
            def test_common(self):
                pass

        class TestBeta:
            def test_common(self):
                pass
    "#,
    );
    fs::write(&test_file, code)?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks);

    let test_alpha = TestItem {
        file: test_file.canonicalize()?,
        function: "test_common".to_string(),
        class: Some("TestAlpha".to_string()),
        line: 2,
    };

    let test_beta = TestItem {
        file: test_file.canonicalize()?,
        function: "test_common".to_string(),
        class: Some("TestBeta".to_string()),
        line: 6,
    };

    // Record coverage for TestAlpha.test_common only
    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![2, 3]);
    depdb.record_test_coverage(&test_alpha, &coverage, true, &block_index);

    let decision_alpha = depdb.needs_run(&test_alpha);
    let decision_beta = depdb.needs_run(&test_beta);

    assert!(
        matches!(decision_alpha, TestRunDecision::CanSkip),
        "TestAlpha.test_common should skip"
    );
    assert!(
        matches!(decision_beta, TestRunDecision::NeverRun),
        "TestBeta.test_common should be NeverRun"
    );

    Ok(())
}

// =============================================================================
// Persistence Tests
// =============================================================================

#[test]
fn save_and_load_roundtrip() -> Result<()> {
    // This test requires setting up a temp cache directory
    // For now, just verify the basic API works

    let mut depdb = DependencyDatabase::default();

    // Add some data
    let test = TestItem {
        file: PathBuf::from("/tmp/test_foo.py"),
        function: "test_ok".to_string(),
        class: None,
        line: 1,
    };

    let block_index = HashMap::new();
    depdb.record_test_coverage(&test, &HashMap::new(), true, &block_index);

    // Save and load would normally persist to disk
    // Just verify it doesn't panic
    depdb.save();

    Ok(())
}

#[test]
fn stats_accurate() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    fs::write(&test_file, "def test_a(): pass\ndef test_b(): pass\n")?;

    let mut depdb = DependencyDatabase::default();
    let block_index = HashMap::new();

    let test_a = TestItem {
        file: test_file.clone(),
        function: "test_a".to_string(),
        class: None,
        line: 1,
    };

    let test_b = TestItem {
        file: test_file.clone(),
        function: "test_b".to_string(),
        class: None,
        line: 2,
    };

    // Record test_a as passed, test_b as failed
    depdb.record_test_coverage(&test_a, &HashMap::new(), true, &block_index);
    depdb.record_test_coverage(&test_b, &HashMap::new(), false, &block_index);

    let stats = depdb.stats();

    assert_eq!(stats.total_tests, 2);
    assert_eq!(stats.passed_tests, 1);
    assert_eq!(stats.failed_tests, 1);

    Ok(())
}

// =============================================================================
// Coverage Mapping Edge Cases
// =============================================================================

#[test]
fn coverage_for_file_not_in_block_index_ignored() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");
    fs::write(&test_file, "def test_ok(): pass\n")?;

    let mut depdb = DependencyDatabase::default();

    // Empty block index - no files indexed
    let block_index = HashMap::new();

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "test_ok".to_string(),
        class: None,
        line: 1,
    };

    // Record coverage for a file that's not in the index
    let mut coverage = HashMap::new();
    coverage.insert(PathBuf::from("/some/other/file.py"), vec![1, 2, 3]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Should not panic, and test should be recorded as passed
    // But with no dependencies tracked
    let stats = depdb.stats();
    assert_eq!(stats.total_tests, 1);

    Ok(())
}

#[test]
fn coverage_for_line_not_in_any_block_handled() -> Result<()> {
    let tmp = TempDir::new()?;
    let test_file = tmp.path().join("test_foo.py");

    // Create a file with a blank line that won't be in any block
    let code = "def foo(): pass\n\n\ndef bar(): pass\n";
    fs::write(&test_file, code)?;

    let mut depdb = DependencyDatabase::default();

    let file_blocks = FileBlocks::from_file(&test_file)?;
    depdb.update_blocks(&file_blocks);

    let mut block_index = HashMap::new();
    block_index.insert(test_file.canonicalize()?, file_blocks);

    let test = TestItem {
        file: test_file.canonicalize()?,
        function: "bar".to_string(),
        class: None,
        line: 4,
    };

    // Coverage includes line 2-3 which are blank (not in any block)
    let mut coverage = HashMap::new();
    coverage.insert(test_file.canonicalize()?, vec![2, 3, 4]);
    depdb.record_test_coverage(&test, &coverage, true, &block_index);

    // Should not panic
    let stats = depdb.stats();
    assert_eq!(stats.total_tests, 1);

    Ok(())
}
