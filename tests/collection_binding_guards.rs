use std::fs;
use taut::discovery::{TestItem, extract_tests_from_file};
use tempfile::TempDir;

fn collect(source: &str) -> anyhow::Result<Vec<TestItem>> {
    let dir = TempDir::new()?;
    let file = dir.path().join("test_bindings.py");
    fs::write(&file, source)?;
    extract_tests_from_file(&file)
}

#[test]
fn conditional_class_rebinding_cannot_hide_a_failing_test() {
    for source in [
        "class TestBroken:\n    def test_broken(self): assert False\nif False:\n    TestBroken = None\ndef test_ok(): pass\n",
        "class Base:\n    def test_broken(self): assert False\nTestBroken = Base\nif False:\n    TestBroken = None\ndef test_ok(): pass\n",
        "from unittest import TestCase\nclass BrokenCase(TestCase):\n    def test_broken(self): assert False\nif False:\n    BrokenCase = None\ndef test_ok(): pass\n",
        "class Base:\n    def test_broken(self): assert False\nif False:\n    Base = None\nTestBroken = Base\ndef test_ok(): pass\n",
        "class TestBroken:\n    def test_broken(self): assert False\nfor unused in []:\n    TestBroken = None\ndef test_ok(): pass\n",
    ] {
        let error = collect(source).unwrap_err().to_string();
        assert!(error.contains("conditional rebinding"), "{error}");
        assert!(error.contains("test_bindings.py:"), "{error}");
    }
}

#[test]
fn a_definite_final_binding_resolves_prior_class_uncertainty() {
    let items = collect("class Base:\n    def test_broken(self): assert False\nclass TestBroken(Base): pass\nif flag:\n    TestBroken = None\nTestBroken = Base\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].class.as_deref(), Some("TestBroken"));
    assert_eq!(items[0].function, "test_broken");
}

#[test]
fn definite_class_removal_and_unrelated_helper_uncertainty_are_allowed() {
    let items = collect("class TestRemoved:\n    def test_broken(self): assert False\nif flag:\n    TestRemoved = None\nTestRemoved = None\nclass Helper: pass\nif flag:\n    Helper = None\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}

#[test]
fn known_callable_test_aliases_are_errors_instead_of_partial_success() {
    for source in [
        "def helper(): assert False\ntest_broken = helper\ndef test_ok(): pass\n",
        "async def helper(): assert False\ntest_broken = helper\ndef test_ok(): pass\n",
        "def helper(): assert False\nfirst = helper\nsecond = first\n_test_broken = second\ndef test_ok(): pass\n",
        "test_broken = lambda: 1 / 0\ndef test_ok(): pass\n",
        "def helper(): assert False\ntest_broken: object = helper\ndef test_ok(): pass\n",
        "def helper(): assert False\nif True:\n    test_broken = helper\ndef test_ok(): pass\n",
    ] {
        let error = collect(source).unwrap_err().to_string();
        assert!(error.contains("callable alias"), "{error}");
        assert!(error.contains("test_bindings.py:"), "{error}");
    }
}

#[test]
fn fixture_values_and_non_test_callable_aliases_remain_valid() {
    let items = collect("test_count = 3\ntest_data = {'value': 1}\npayload = [1, 2]\ntest_payload = payload\ndef helper(): pass\nhelper_alias = helper\nhelper = 7\ntest_number = helper\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}
