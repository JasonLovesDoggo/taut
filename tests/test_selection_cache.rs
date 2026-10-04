//! Cache regressions use isolated source trees without writing shared cache state.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use taut::discovery::TestItem;
use taut::runner::{TestCoverage, TestResult};
use taut::selection::TestSelector;
use tempfile::TempDir;

fn test_item(path: &Path, function: &str) -> TestItem {
    TestItem {
        file: path.to_path_buf(),
        function: function.to_string(),
        class: None,
        line: 1,
        markers: vec![],
    }
}

fn record(selector: &mut TestSelector, item: &TestItem, files: Vec<PathBuf>) {
    selector.record_result(&TestResult {
        item: item.clone(),
        passed: true,
        duration: Duration::ZERO,
        error: None,
        skipped: false,
        skip_reason: None,
        coverage: Some(TestCoverage {
            files: files.into_iter().map(|file| (file, vec![1])).collect(),
        }),
        stdout: None,
        stderr: None,
    });
}

fn project() -> (TempDir, PathBuf, TestItem) {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pyproject.toml"),
        "[project]\nname = 'example'\n",
    )
    .unwrap();
    let tests = tmp.path().join("tests");
    fs::create_dir(&tests).unwrap();
    let path = tests.join("test_example.py");
    fs::write(&path, "def test_example(): pass\n").unwrap();
    let test = test_item(&path, "test_example");
    (tmp, tests, test)
}

#[test]
fn tests_directory_selection_tracks_sibling_application_sources() {
    let (tmp, tests, test) = project();
    let helper = tmp.path().join("application.py");
    fs::write(&helper, "VALUE = 1\n").unwrap();
    let mut selector = TestSelector::new();
    selector.index_files(std::slice::from_ref(&tests));
    // Cached imports can omit the helper from this test's coverage entirely.
    record(&mut selector, &test, vec![test.file.clone()]);
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .skip_count(),
        1
    );
    fs::write(&helper, "VALUE = 2\n").unwrap();
    selector.index_files(&[tests]);
    assert_eq!(selector.select_tests(&[test]).run_count(), 1);
}

#[test]
fn source_addition_and_deletion_invalidate_all_tests() {
    let (tmp, tests, test) = project();
    let mut selector = TestSelector::new();
    selector.index_files(std::slice::from_ref(&tests));
    record(&mut selector, &test, vec![test.file.clone()]);
    let helper = tmp.path().join("new_module.py");
    fs::write(&helper, "VALUE = 1\n").unwrap();
    selector.index_files(std::slice::from_ref(&tests));
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .run_count(),
        1
    );
    record(&mut selector, &test, vec![test.file.clone()]);
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .skip_count(),
        1
    );
    fs::remove_file(helper).unwrap();
    selector.index_files(&[tests]);
    assert_eq!(selector.select_tests(&[test]).run_count(), 1);
}

#[test]
fn external_import_is_shared_even_when_later_coverage_omits_it() {
    let (_tmp, tests, first) = project();
    let second = test_item(&first.file, "test_other");
    let external = TempDir::new().unwrap();
    let helper = external.path().join("helper.py");
    fs::write(&helper, "VALUE = 1\n").unwrap();
    let mut selector = TestSelector::new();
    selector.index_files(std::slice::from_ref(&tests));
    record(
        &mut selector,
        &first,
        vec![first.file.clone(), helper.clone()],
    );
    record(&mut selector, &second, vec![second.file.clone()]);
    let items = [first, second];
    // The first trace discovered an external dependency after execution.
    // Establish its pre-run fingerprint before caching a successful run.
    assert_eq!(selector.select_tests(&items).run_count(), 2);
    selector.index_files(std::slice::from_ref(&tests));
    for test in &items {
        record(&mut selector, test, vec![test.file.clone()]);
    }
    assert_eq!(selector.select_tests(&items).skip_count(), 2);
    fs::write(&helper, "VALUE = 2\n").unwrap();
    selector.index_files(std::slice::from_ref(&tests));
    assert_eq!(selector.select_tests(&items).run_count(), 2);
    for test in &items {
        record(&mut selector, test, vec![test.file.clone()]);
    }
    fs::remove_file(helper).unwrap();
    selector.index_files(&[tests]);
    assert_eq!(selector.select_tests(&items).run_count(), 2);
}

#[test]
fn empty_or_unreadable_coverage_never_certifies_a_cached_pass() {
    let (_tmp, tests, test) = project();
    let mut selector = TestSelector::new();
    selector.index_files(&[tests]);
    record(&mut selector, &test, vec![]);
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .run_count(),
        1
    );
    let missing = test.file.with_file_name("missing.py");
    record(&mut selector, &test, vec![test.file.clone(), missing]);
    assert_eq!(selector.select_tests(&[test]).run_count(), 1);
}

#[test]
fn execution_context_changes_invalidate_cached_passes() {
    let (_tmp, tests, test) = project();
    let mut selector = TestSelector::new();
    selector.set_execution_context("python-a|process");
    selector.index_files(&[tests]);
    record(&mut selector, &test, vec![test.file.clone()]);
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .skip_count(),
        1
    );
    selector.set_execution_context("python-b|process");
    assert_eq!(selector.select_tests(&[test]).run_count(), 1);
}

#[test]
fn removed_and_shifted_blocks_do_not_remain_in_the_database() {
    use taut::blocks::FileBlocks;
    use taut::depdb::DependencyDatabase;
    let (_tmp, _tests, test) = project();
    fs::write(
        &test.file,
        "def test_example(): pass\ndef removed(): return 1\n",
    )
    .unwrap();
    let mut db = DependencyDatabase::default();
    db.update_blocks(&FileBlocks::from_file(&test.file).unwrap());
    assert_eq!(db.stats().total_blocks, 2);
    db.record_test_coverage(
        &test,
        &HashMap::from([(test.file.clone(), vec![1])]),
        true,
        &HashMap::new(),
    );
    fs::write(&test.file, "\ndef test_example(): pass\n").unwrap();
    db.update_blocks(&FileBlocks::from_file(&test.file).unwrap());
    assert_eq!(db.stats().total_blocks, 1);
    assert!(db.needs_run(&test).should_run());
}

#[test]
fn external_source_changed_after_import_cannot_certify_new_bytes() {
    let (_tmp, tests, test) = project();
    let external = TempDir::new().unwrap();
    let helper = external.path().join("helper.py");
    fs::write(&helper, "VALUE = 1\n").unwrap();
    let mut selector = TestSelector::new();
    selector.index_files(std::slice::from_ref(&tests));

    // Python imported VALUE = 1 and the test passed. Before the result reaches
    // the cache, the source changes to code that would fail the next run.
    fs::write(&helper, "VALUE = 2\n").unwrap();
    record(
        &mut selector,
        &test,
        vec![test.file.clone(), helper.clone()],
    );
    selector.index_files(std::slice::from_ref(&tests));
    assert_eq!(
        selector
            .select_tests(std::slice::from_ref(&test))
            .run_count(),
        1
    );

    // Once the external file is indexed before execution, stable source can
    // produce a reusable pass, including for tests sharing the import cache.
    record(&mut selector, &test, vec![test.file.clone(), helper]);
    selector.index_files(&[tests]);
    assert_eq!(selector.select_tests(&[test]).skip_count(), 1);
}
