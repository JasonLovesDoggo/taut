use serde_json::json;
use std::{collections::HashSet, fs, path::PathBuf};
use taut::discovery::{TestItem, collect_tests, extract_tests_from_file};
use tempfile::TempDir;

fn fixture(source: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test_parameters.py");
    fs::write(&path, source).unwrap();
    (dir, path)
}

fn collect(source: &str) -> Vec<TestItem> {
    let (_dir, path) = fixture(source);
    extract_tests_from_file(&path).unwrap()
}

#[test]
fn literal_cases_preserve_values_and_original_function() {
    let tests = collect(
        r#"
@taut.parametrize("value", [None, True, False, "hi", -2, +4, 1.25, [1, None], {"nested": [1, False]}])
def test_value(value): pass
"#,
    );
    assert_eq!(tests.len(), 9);
    assert!(tests.iter().all(|test| test.function == "test_value"));
    let expected = vec![
        json!(null),
        json!(true),
        json!(false),
        json!("hi"),
        json!(-2),
        json!(4),
        json!(1.25),
        json!([1, null]),
        json!({"nested": [1, false]}),
    ];
    for (test, value) in tests.iter().zip(expected) {
        assert_eq!(test.parameters(), Some(json!({"value": value})));
    }
    assert!(tests[0].id().ends_with("::test_value[None]"));
}

#[test]
fn tuple_containers_rows_and_keyword_arguments() {
    let tests = collect(
        r#"
@pytest.mark.parametrize(argnames=("a", "b"), argvalues=((1, "x"), (2, "y")), ids=["first", "second"])
async def test_pair(a, b): pass
"#,
    );
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].parameters(), Some(json!({"a": 1, "b": "x"})));
    assert!(tests[1].id().ends_with("::test_pair[second]"));
}

#[test]
fn single_name_sequence_uses_rows_like_pytest() {
    let tests = collect("@parametrize(['value'], [(1,), (2,)])\ndef test_one(value): pass\n");
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].parameters(), Some(json!({"value": 1})));
}

#[test]
fn stacked_cases_are_cartesian_and_deterministic() {
    let source = r#"
@parametrize("x", [1, 2])
@parametrize("y", ["a", "b"])
def test_product(x, y): pass
"#;
    let tests = collect(source);
    assert_eq!(tests.len(), 4);
    let ids: Vec<_> = tests
        .iter()
        .map(|test| test.id().split("::").last().unwrap().to_owned())
        .collect();
    assert_eq!(
        ids,
        [
            "test_product[a-1]",
            "test_product[a-2]",
            "test_product[b-1]",
            "test_product[b-2]"
        ]
    );
    assert_eq!(tests[2].parameters(), Some(json!({"x": 1, "y": "b"})));
}

#[test]
fn duplicate_ids_are_unique_and_collision_safe() {
    let tests = collect(
        "@parametrize('x', [1, 2, 3], ids=['same', 'same', 'same-0'])\ndef test_ids(x): pass\n",
    );
    let ids: Vec<_> = tests.iter().map(TestItem::id).collect();
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 3);
    assert!(ids[0].ends_with("[same-1]"));
    assert!(ids[1].ends_with("[same-2]"));
    assert!(ids[2].ends_with("[same-0]"));
}

#[test]
fn class_and_method_parameters_combine_and_retain_markers() {
    let tests = collect(
        r#"
@parallel
@parametrize("outer", [1, 2])
class TestCases:
    @parametrize("inner", ["a", "b"])
    async def test_method(self, outer, inner): pass
"#,
    );
    assert_eq!(tests.len(), 4);
    assert!(tests.iter().all(|test| test.is_parallel()));
    assert_eq!(tests[0].class.as_deref(), Some("TestCases"));
    assert_eq!(
        tests[3].parameters(),
        Some(json!({"outer": 2, "inner": "b"}))
    );
}

#[test]
fn exact_selectors_and_filters_match_cases() {
    let (_dir, path) = fixture(
        r#"
@parametrize("x", [1, 2], ids=["fast", "slow"])
def test_item(x): pass
class TestItems:
    @parametrize("x", [1, 2], ids=["fast", "slow"])
    def test_method(self, x): pass
"#,
    );
    let selected = collect_tests(
        &[PathBuf::from(format!(
            "{}::test_item[slow]",
            path.display()
        ))],
        None,
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].parameters(), Some(json!({"x": 2})));
    let selected = collect_tests(
        &[PathBuf::from(format!(
            "{}::TestItems::test_method[fast]",
            path.display()
        ))],
        None,
    )
    .unwrap();
    assert_eq!(selected.len(), 1);
    let all_function_cases = collect_tests(
        &[PathBuf::from(format!("{}::test_item", path.display()))],
        None,
    )
    .unwrap();
    assert_eq!(all_function_cases.len(), 2);
    assert_eq!(
        collect_tests(&[path.clone()], Some("[slow]"))
            .unwrap()
            .len(),
        2
    );
    assert!(
        collect_tests(
            &[PathBuf::from(format!(
                "{}::test_item[missing]",
                path.display()
            ))],
            None
        )
        .is_err()
    );
}

#[test]
fn separator_and_control_characters_are_safe_in_case_ids() {
    let (_dir, path) =
        fixture("@parametrize('x', ['value'], ids=['a::b[c]/d\\n?*'])\ndef test_safe(x): pass\n");
    let tests = extract_tests_from_file(&path).unwrap();
    assert!(tests[0].id().ends_with("[a%3A%3Ab%5Bc%5D%2Fd%0A%3F%2A]"));
    assert_eq!(
        collect_tests(&[PathBuf::from(tests[0].id())], None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn malformed_or_unsupported_sets_fail_collection_with_context() {
    for (decorator, expected) in [
        ("@parametrize('x', [])", "empty parameter set"),
        ("@parametrize('a,b', [(1,)])", "1 values for 2 names"),
        ("@parametrize('x', build_values())", "dynamic values"),
        ("@parametrize('x', [object()])", "dynamic parameter"),
        ("@parametrize('x', [(1, 2)])", "tuple-valued"),
        ("@parametrize('x', [[(1, 2)]])", "tuple-valued"),
        ("@parametrize('x', [{1: 'value'}])", "string keys"),
        ("@parametrize('x', [1], ids=[])", "ids count"),
        ("@parametrize('x', [1], indirect=True)", "indirect"),
        ("@parametrize('x,x', [(1, 2)])", "duplicate parameter name"),
        (
            "@parametrize('x', [1])\n@parametrize('x', [2])",
            "stacked decorators",
        ),
        ("@pytest.mark.parametrize", "must be called"),
    ] {
        let (_dir, path) = fixture(&format!("{decorator}\ndef test_broken(x): pass\n"));
        let message = format!("{:#}", extract_tests_from_file(&path).unwrap_err());
        assert!(message.contains(expected), "{decorator}: {message}");
        assert!(message.contains("test_parameters.py:"), "{message}");
    }
}

#[test]
fn unparameterized_tests_keep_their_identity() {
    let tests = collect("@parallel\ndef test_plain(): pass\n");
    assert!(tests[0].parameters().is_none());
    assert!(tests[0].id().ends_with("::test_plain"));
    assert!(tests[0].is_parallel());
}

#[test]
fn nested_dictionary_insertion_order_is_preserved() {
    let tests = collect(
        "@parametrize('value', [{'b': {'z': 1, 'a': 2}, 'a': 3}])\ndef test_dict(value): pass\n",
    );
    let values = tests[0].parameters().unwrap();
    let dictionary = values["value"].as_object().unwrap();
    assert_eq!(
        dictionary.keys().map(String::as_str).collect::<Vec<_>>(),
        ["b", "a"]
    );
    let nested = dictionary["b"].as_object().unwrap();
    assert_eq!(
        nested.keys().map(String::as_str).collect::<Vec<_>>(),
        ["z", "a"]
    );
}

#[test]
fn none_ids_and_integer_boundaries_preserve_python_values() {
    let tests = collect(
        "@parametrize('x', [-9223372036854775808, 18446744073709551615], ids=None)\ndef test_limits(x): pass\n",
    );
    assert_eq!(tests[0].parameters(), Some(json!({"x": i64::MIN})));
    assert_eq!(tests[1].parameters(), Some(json!({"x": u64::MAX})));
    let (_dir, path) =
        fixture("@parametrize('x', [18446744073709551616])\ndef test_too_large(x): pass\n");
    assert!(format!("{:#}", extract_tests_from_file(&path).unwrap_err()).contains("64 bits"));
}

#[test]
fn cartesian_expansion_is_bounded_before_allocating_product() {
    let values = (0..317)
        .map(|number| number.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let (_dir, path) = fixture(&format!(
        "@parametrize('x', [{values}])\n@parametrize('y', [{values}])\ndef test_product(x, y): pass\n"
    ));
    assert!(
        format!("{:#}", extract_tests_from_file(&path).unwrap_err())
            .contains("exceeds 100000 cases")
    );
}

#[test]
fn class_parameters_expand_inherited_methods_and_aliases() {
    let tests = collect(
        r#"
class Base:
    @parametrize("inner", ["a", "b"])
    def test_method(self, outer, inner): pass
@parametrize("outer", [1, 2])
class TestDerived(Base): pass
TestAlias = TestDerived
"#,
    );
    assert_eq!(tests.len(), 8);
    for name in ["TestDerived", "TestAlias"] {
        let cases = tests
            .iter()
            .filter(|test| test.class.as_deref() == Some(name))
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), 4);
        assert!(cases.iter().all(|test| test.function == "test_method"));
        assert_eq!(
            cases[0].parameters(),
            Some(json!({"inner": "a", "outer": 1}))
        );
        assert_eq!(
            cases[3].parameters(),
            Some(json!({"inner": "b", "outer": 2}))
        );
    }
}

#[test]
fn base_class_parameters_follow_mro_and_overridden_methods() {
    let tests = collect(
        r#"
@parametrize("value", [1, 2])
class Base:
    @parametrize("stale", [0])
    def test_method(self, value, stale): pass
class TestDerived(Base):
    def test_method(self, value): pass
"#,
    );
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].parameters(), Some(json!({"value": 1})));
    assert_eq!(tests[1].parameters(), Some(json!({"value": 2})));
}

#[test]
fn inherited_unittest_run_test_parameters_expand_only_when_used() {
    let tests = collect(
        r#"
import unittest
class Base(unittest.TestCase):
    @parametrize("value", [1, 2])
    def runTest(self, value): pass
class Child(Base): pass
class Named(Base):
    def test_named(self): pass
"#,
    );
    assert_eq!(tests.len(), 5);
    assert_eq!(
        tests
            .iter()
            .filter(|test| test.class.as_deref() == Some("Child"))
            .count(),
        2
    );
    let named = tests
        .iter()
        .filter(|test| test.class.as_deref() == Some("Named"))
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 1);
    assert!(named[0].parameters().is_none());
    assert_eq!(named[0].function, "test_named");
}
