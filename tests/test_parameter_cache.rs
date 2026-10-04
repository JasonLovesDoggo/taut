//! Parameter cases must never overwrite another case's cached failure or pass.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Duration;
use taut::blocks::FileBlocks;
use taut::depdb::{DependencyDatabase, TestId, TestRunDecision};
use taut::discovery::{TestItem, extract_tests_from_file};
use taut::parametrize::case_id;
use taut::runner::{TestCoverage, TestResult};
use taut::selection::TestSelector;
use tempfile::TempDir;

fn cases() -> (TempDir, Vec<TestItem>) {
    let root = TempDir::new().unwrap();
    let file = root.path().join("test_parameters.py");
    fs::write(
        root.path().join("pyproject.toml"),
        "[project]\nname='case-cache'\n",
    )
    .unwrap();
    fs::write(&file, "@parametrize('value', [0, 1], ids=['fails', 'passes'])\ndef test_value(value):\n    assert value == 1\n").unwrap();
    let cases = extract_tests_from_file(&file).unwrap();
    assert_eq!(cases.len(), 2);
    assert_eq!(case_id(&cases[0]), Some("fails"));
    assert_eq!(case_id(&cases[1]), Some("passes"));
    (root, cases)
}

#[test]
fn passing_later_case_cannot_overwrite_failure_or_its_persisted_identity() {
    let (_root, cases) = cases();
    let mut database = DependencyDatabase::default();
    database.update_blocks(&FileBlocks::from_file(&cases[0].file).unwrap());
    let coverage = HashMap::from([(cases[0].file.clone(), vec![3])]);
    database.record_test_coverage(&cases[0], &coverage, false, &HashMap::new());
    database.record_test_coverage(&cases[1], &coverage, true, &HashMap::new());

    assert!(matches!(
        database.needs_run(&cases[0]),
        TestRunDecision::FailedLastTime
    ));
    assert!(matches!(
        database.needs_run(&cases[1]),
        TestRunDecision::CanSkip
    ));
    assert_eq!(database.stats().total_tests, 2);
    assert_eq!(database.stats().failed_tests, 1);
    assert_eq!(database.stats().passed_tests, 1);

    let encoded = serde_json::to_value(&database).unwrap();
    let stored: HashSet<TestId> = encoded["tests"]
        .as_object()
        .unwrap()
        .keys()
        .map(|key| serde_json::from_str(key).unwrap())
        .collect();
    let expected: HashSet<TestId> = cases.iter().map(TestId::from).collect();
    assert_eq!(
        stored, expected,
        "cached-path and direct identities must agree"
    );
    assert_eq!(expected.len(), 2);
    assert!(cases.iter().all(|case| case.function == "test_value"));

    let mut restored: DependencyDatabase = serde_json::from_value(encoded).unwrap();
    restored.update_blocks(&FileBlocks::from_file(&cases[0].file).unwrap());
    assert!(matches!(
        restored.needs_run(&cases[0]),
        TestRunDecision::FailedLastTime
    ));
    assert!(matches!(
        restored.needs_run(&cases[1]),
        TestRunDecision::CanSkip
    ));
}

#[test]
fn changed_selection_reruns_failed_case_and_skips_only_the_passing_case() {
    let (_root, cases) = cases();
    let mut selector = TestSelector::new();
    selector.index_files(std::slice::from_ref(&cases[0].file));
    for (index, case) in cases.iter().enumerate() {
        selector.record_result(&TestResult {
            item: case.clone(),
            passed: index == 1,
            duration: Duration::ZERO,
            error: None,
            skipped: false,
            skip_reason: None,
            stdout: None,
            stderr: None,
            coverage: Some(TestCoverage {
                files: HashMap::from([(case.file.clone(), vec![3])]),
            }),
        });
    }
    selector.index_files(std::slice::from_ref(&cases[0].file));
    let selected = selector.select_tests(&cases);
    assert_eq!(
        selected.run_count(),
        1,
        "a later passing case must never hide a failure"
    );
    assert_eq!(selected.skip_count(), 1);
    assert_eq!(case_id(&selected.to_run[0].0), Some("fails"));
    assert_eq!(case_id(&selected.to_skip[0].0), Some("passes"));
}
