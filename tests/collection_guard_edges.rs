use std::fs;
use taut::discovery::extract_tests_from_file;
use tempfile::TempDir;

fn collect(source: &str) -> anyhow::Result<Vec<taut::discovery::TestItem>> {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("test_guards.py");
    fs::write(&file, source).unwrap();
    extract_tests_from_file(&file)
}

#[test]
fn conditional_class_rebinding_cannot_hide_a_failing_test() {
    for condition in ["True", "False", "flag"] {
        let source = format!(
            "class TestBroken:\n    def test_broken(self): assert False\nif {condition}:\n    TestBroken = None\ndef test_ok(): pass\n"
        );
        let error = collect(&source).unwrap_err().to_string();
        assert!(
            error.contains("TestBroken") && error.contains("rebinding"),
            "{error}"
        );
        assert!(error.contains("test_guards.py:3"), "{error}");
    }
}

#[test]
fn conditional_class_alias_rebinding_cannot_hide_inherited_tests() {
    let source = "class Base:\n    def test_broken(self): assert False\nTestBroken = Base\nif False:\n    TestBroken = None\ndef test_ok(): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("rebinding")
    );
}

#[test]
fn conditional_unittest_rebinding_is_rejected_without_a_test_class_prefix() {
    let source = "import unittest\nclass Behaviour(unittest.TestCase):\n    def testBroken(self): assert False\nif False:\n    Behaviour = None\ndef test_ok(): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("Behaviour")
    );
}

#[test]
fn class_deletion_and_loop_target_rebinding_are_uncertain() {
    for operation in [
        "if False:\n    del TestBroken\n",
        "for TestBroken in []:\n    pass\n",
    ] {
        let source = format!(
            "class TestBroken:\n    def test_broken(self): assert False\n{operation}def test_ok(): pass\n"
        );
        assert!(
            collect(&source)
                .unwrap_err()
                .to_string()
                .contains("rebinding")
        );
    }
}

#[test]
fn direct_non_callable_class_replacement_still_intentionally_removes_the_class() {
    let items = collect("class TestRemoved:\n    def test_broken(self): assert False\nTestRemoved = None\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}

#[test]
fn module_callable_aliases_are_explicitly_unsupported() {
    for source in [
        "def helper(): assert False\ntest_broken = helper\ndef test_ok(): pass\n",
        "async def helper(): assert False\nalias = helper\n_test_broken = alias\n",
        "test_broken = lambda: 1 / 0\n",
        "def helper(): assert False\ntest_broken: object = helper\n",
        "def helper(): assert False\nhelper, test_broken = None, helper\n",
        "def helper(): assert False\n[test_broken] = (helper,)\n",
        "def helper(): assert False\nif False:\n    helper = None\ntest_broken = helper\n",
        "def helper(): assert False\nif False:\n    test_broken = helper\n",
    ] {
        let error = collect(source).unwrap_err().to_string();
        assert!(
            error.contains("callable alias") && error.contains("unsupported"),
            "{error}"
        );
        assert!(error.contains("test_guards.py:"), "{error}");
    }
}

#[test]
fn constant_aliases_and_fixture_values_are_not_test_functions() {
    let items = collect("CONFIG = 1\ntest_number = CONFIG\ntest_config = {'enabled': True}\ntest_values = [1, 2, 3]\ntest_client = make_client()\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}

#[test]
fn callable_names_can_be_replaced_with_constants_or_imported_data() {
    for shadow in [
        "helper = 42",
        "from data import VALUE as helper",
        "import data as helper",
    ] {
        let source = format!(
            "def helper(): assert False\n{shadow}\ntest_data = helper\ndef test_ok(): pass\n"
        );
        assert_eq!(collect(&source).unwrap().len(), 1);
    }
}

#[test]
fn test_function_bodies_are_not_module_aliases() {
    let items =
        collect("def test_ok():\n    helper = lambda: None\n    test_local = helper\n").unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn conditional_creation_of_a_known_class_alias_is_rejected() {
    let source = "class Base:\n    def test_broken(self): assert False\nif True:\n    TestBroken = Base\ndef test_ok(): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("TestBroken")
    );
}

#[test]
fn class_objects_used_as_module_fixture_values_are_not_function_aliases() {
    let source = "class Model: pass\ntest_model_type = Model\ndef test_ok(): pass\n";
    assert_eq!(collect(source).unwrap().len(), 1);
}

#[test]
fn conditional_class_alias_creation_can_be_resolved_by_a_definite_final_binding() {
    let items = collect("class Base:\n    def test_broken(self): assert False\nif flag:\n    TestAlias = Base\nTestAlias = Base\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].class.as_deref(), Some("TestAlias"));
    let items = collect("class Base:\n    def test_broken(self): assert False\nif flag:\n    TestAlias = Base\nTestAlias = None\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}

#[test]
fn transitive_conditional_class_aliases_remain_uncertain() {
    let source = "class Base:\n    def test_broken(self): assert False\nif flag:\n    Alias = Base\n    TestAlias = Alias\ndef test_ok(): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("TestAlias")
    );
}

#[test]
fn starred_fixture_containers_are_not_callable_tests() {
    let items =
        collect("def helper(): pass\n[*test_handlers] = [helper]\ndef test_ok(): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].function, "test_ok");
}
