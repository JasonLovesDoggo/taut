use std::fs;
use taut::discovery::{TestItem, extract_tests_from_file};
use tempfile::TempDir;

fn collect(source: &str) -> anyhow::Result<Vec<TestItem>> {
    let dir = TempDir::new()?;
    let file = dir.path().join("test_classes.py");
    fs::write(&file, source)?;
    extract_tests_from_file(&file)
}

fn names(items: &[TestItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| format!("{}::{}", item.class.as_deref().unwrap_or(""), item.function))
        .collect()
}

#[test]
fn inherited_failure_is_collected_on_the_actual_subclass() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nclass TestThing(Base):\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(
        names(&items),
        ["TestThing::test_ok", "TestThing::test_fail"]
    );
    assert_eq!(items[1].line, 2);
}

#[test]
fn overrides_replace_inherited_methods_once() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nclass TestThing(Base):\n    def test_fail(self): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_fail"]);
    assert_eq!(items[0].line, 4);
}

#[test]
fn diamond_uses_c3_instead_of_depth_first_resolution() {
    let items = collect("class Common:\n    def test_choice(self): assert False\nclass Left(Common): pass\nclass Right(Common):\n    def test_choice(self): pass\nclass TestDiamond(Left, Right): pass\n").unwrap();
    assert_eq!(names(&items), ["TestDiamond::test_choice"]);
    assert_eq!(items[0].line, 5);
}

#[test]
fn multiple_inheritance_preserves_left_to_right_precedence() {
    let items = collect("class Left:\n    def test_choice(self): pass\nclass Right:\n    def test_choice(self): assert False\nclass TestChoice(Left, Right): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].line, 2);
}

#[test]
fn non_callable_attributes_shadow_inherited_tests() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nclass TestThing(Base):\n    test_fail = None\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_ok"]);
}

#[test]
fn same_class_final_binding_wins() {
    let items = collect("class TestThing:\n    def test_hidden(self): assert False\n    test_hidden = None\n    test_visible = None\n    def test_visible(self): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_visible"]);
    assert_eq!(items[0].line, 5);
}

#[test]
fn annotation_without_assignment_does_not_shadow() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nclass TestThing(Base):\n    test_fail: object\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_fail"]);
}

#[test]
fn deleting_override_reveals_inherited_method() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nclass TestThing(Base):\n    test_fail = None\n    del test_fail\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_fail"]);
}

#[test]
fn inherited_async_methods_are_collected() {
    let items = collect(
        "class Base:\n    async def test_fail(self): assert False\nclass TestThing(Base): pass\n",
    )
    .unwrap();
    assert_eq!(names(&items), ["TestThing::test_fail"]);
}

#[test]
fn local_base_aliases_work() {
    let items = collect("class Base:\n    def test_fail(self): assert False\nAlias = Base\nclass TestThing(Alias): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_fail"]);
}

#[test]
fn unittest_classes_do_not_require_test_class_names() {
    let items = collect("import unittest\nclass ArithmeticCase(unittest.TestCase):\n    def testFail(self): assert False\n").unwrap();
    assert_eq!(names(&items), ["ArithmeticCase::testFail"]);
}

#[test]
fn unittest_alias_forms_are_recognized() {
    for source in [
        "import unittest as ut\nclass Case(ut.TestCase):\n    def test_fail(self): assert False\n",
        "from unittest import TestCase as TC\nclass Case(TC):\n    def test_fail(self): assert False\n",
        "from unittest.case import TestCase as TC\nclass Case(TC):\n    def test_fail(self): assert False\n",
        "import unittest.case as cases\nclass Case(cases.TestCase):\n    def test_fail(self): assert False\n",
        "from unittest import case as cases\nclass Case(cases.TestCase):\n    def test_fail(self): assert False\n",
        "import unittest\nBase = unittest.TestCase\nclass Case(Base):\n    def test_fail(self): assert False\n",
    ] {
        assert_eq!(
            names(&collect(source).unwrap()),
            ["Case::test_fail"],
            "{source}"
        );
    }
}

#[test]
fn isolated_asyncio_alias_and_local_descendants_are_recognized() {
    let items = collect("from unittest import IsolatedAsyncioTestCase as AsyncCase\nclass Base(AsyncCase):\n    async def testFail(self): assert False\nclass Child(Base): pass\n").unwrap();
    assert_eq!(names(&items), ["Base::testFail", "Child::testFail"]);
}

#[test]
fn unittest_and_mixin_mro_collects_all_methods() {
    let items = collect("import unittest\nclass Mixin:\n    def test_fail(self): assert False\nclass Case(Mixin, unittest.TestCase):\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(names(&items), ["Case::test_ok", "Case::test_fail"]);
}

#[test]
fn unknown_external_or_dynamic_test_bases_cannot_hide_tests() {
    for source in [
        "from other import Base\nclass TestThing(Base):\n    def test_ok(self): pass\n",
        "from other import Base\nclass TestThing(Base): pass\n",
        "from other import Base\nclass Local(Base):\n    def test_fail(self): assert False\nclass TestThing(Local): pass\n",
        "class TestThing(factory()):\n    def test_ok(self): pass\n",
        "from other import TestCase\nclass Case(TestCase):\n    def test_fail(self): assert False\n",
    ] {
        let error = collect(source).unwrap_err().to_string();
        assert!(error.contains("unsupported inheritance"), "{error}");
    }
}

#[test]
fn nested_tests_cannot_be_silently_omitted() {
    for source in [
        "class TestOuter:\n    def test_ok(self): pass\n    class TestInner:\n        def test_fail(self): assert False\n",
        "class Base:\n    class TestInner:\n        def test_fail(self): assert False\nclass TestOuter(Base): pass\n",
        "import unittest\nclass Outer(unittest.TestCase):\n    class Inner(unittest.TestCase):\n        def testFail(self): assert False\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("nested test class")
        );
    }
}

#[test]
fn inconsistent_or_duplicate_bases_fail_collection() {
    for source in [
        "class A: pass\nclass B: pass\nclass Left(A, B): pass\nclass Right(B, A): pass\nclass TestThing(Left, Right): pass\n",
        "class A: pass\nclass TestThing(A, A): pass\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("method resolution order")
        );
    }
}

#[test]
fn dynamic_test_attributes_are_errors_not_empty_successes() {
    for source in [
        "class TestThing:\n    test_fail = lambda self: 1 / 0\n",
        "class Base:\n    def test_fail(self): assert False\nclass TestThing:\n    test_fail = Base.test_fail\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("dynamic test attribute")
        );
    }
}

#[test]
fn overwritten_classes_do_not_duplicate_tests() {
    let items = collect("class TestThing:\n    def test_old(self): assert False\nclass TestThing:\n    def test_new(self): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_new"]);
}

#[test]
fn unrelated_external_helper_classes_do_not_block_tests() {
    let items = collect("from other import Base\nclass Helper(Base):\n    def helper(self): pass\nclass TestThing:\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(names(&items), ["TestThing::test_ok"]);
}

#[test]
fn test_class_aliases_use_the_module_binding_name() {
    let items =
        collect("class Base:\n    def test_fail(self): assert False\nTestAlias = Base\n").unwrap();
    assert_eq!(names(&items), ["TestAlias::test_fail"]);
}

#[test]
fn conditional_unittest_camelcase_tests_are_rejected() {
    for source in [
        "import unittest\nclass Case(unittest.TestCase):\n    if True:\n        def testFail(self): assert False\n",
        "import unittest\nif True:\n    class Case(unittest.TestCase):\n        def testFail(self): assert False\n",
        "import unittest\nif True:\n    class Case(unittest.TestCase):\n        def runTest(self): assert False\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("compound statements")
        );
    }
}

#[test]
fn destructured_callable_bindings_cannot_be_ignored() {
    let source = "class TestThing:\n    test_fail, helper = (lambda self: 1 / 0, None)\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("dynamic test attribute")
    );
}

#[test]
fn unittest_run_test_fallback_is_not_omitted() {
    let items = collect(
        "import unittest\nclass Case(unittest.TestCase):\n    def runTest(self): assert False\n",
    )
    .unwrap();
    assert_eq!(names(&items), ["Case::runTest"]);
    let items = collect("import unittest\nclass Base:\n    def runTest(self): assert False\nclass Case(Base, unittest.TestCase): pass\n").unwrap();
    assert_eq!(names(&items), ["Case::runTest"]);
}

#[test]
fn unittest_run_test_fallback_is_unused_when_named_tests_exist() {
    let items = collect("import unittest\nclass Case(unittest.TestCase):\n    def runTest(self): assert False\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(names(&items), ["Case::test_ok"]);
}

#[test]
fn unused_run_test_fallback_does_not_block_named_tests() {
    let items = collect("import unittest\nclass Case(unittest.TestCase):\n    runTest = None\n    def test_ok(self): pass\n").unwrap();
    assert_eq!(names(&items), ["Case::test_ok"]);
}

#[test]
fn conditional_base_rebinding_cannot_use_a_stale_mro() {
    let source = "class Base:\n    def test_ok(self): pass\nif flag:\n    Base = External\nclass TestThing(Base): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("unsupported inheritance")
    );
}

#[test]
fn conditional_callable_attributes_cannot_be_omitted() {
    let source = "class TestThing:\n    if True:\n        test_fail = lambda self: 1 / 0\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("dynamic test attribute")
    );
}

#[test]
fn conditional_empty_test_subclasses_cannot_hide_inherited_methods() {
    let source = "class Base:\n    def test_fail(self): assert False\nif True:\n    class TestThing(Base): pass\n";
    assert!(
        collect(source)
            .unwrap_err()
            .to_string()
            .contains("compound statements")
    );
}

#[test]
fn conditional_unittest_subclasses_are_classified_through_aliases() {
    for source in [
        "import unittest\nclass Base(unittest.TestCase):\n    def testFail(self): assert False\nif True:\n    class Behaviour(Base): pass\n",
        "from unittest import TestCase as TC\nif True:\n    class Behaviour(TC): pass\n",
        "import unittest as ut\nAlias = ut.TestCase\nif True:\n    class Behaviour(Alias): pass\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("compound statements")
        );
    }
}

#[test]
fn nested_unittest_subclasses_with_only_inherited_tests_are_rejected() {
    for source in [
        "import unittest\nclass Base(unittest.TestCase):\n    def testFail(self): assert False\nclass Outer(unittest.TestCase):\n    class Inner(Base): pass\n",
        "import unittest\nclass Base(unittest.TestCase):\n    def testFail(self): assert False\nclass Outer(unittest.TestCase):\n    if True:\n        class Inner(Base): pass\n",
    ] {
        assert!(
            collect(source)
                .unwrap_err()
                .to_string()
                .contains("nested test class")
        );
    }
}

#[test]
fn class_serial_markers_survive_unrelated_method_marks() {
    use taut::markers::MarkerValue;
    let items = collect("@mark(serial=True, group='database')\nclass TestDatabase:\n    @mark(slow=True)\n    def test_write(self): pass\n").unwrap();
    let marker = items[0]
        .markers
        .iter()
        .find(|marker| marker.name == "mark")
        .unwrap();
    assert_eq!(
        marker.args.kwargs.get("serial"),
        Some(&MarkerValue::Bool(true))
    );
    assert_eq!(
        marker.args.kwargs.get("slow"),
        Some(&MarkerValue::Bool(true))
    );
    assert_eq!(items[0].groups(), ["database"]);
}

#[test]
fn method_explicit_false_overrides_class_defaults_by_key() {
    use taut::markers::MarkerValue;
    let items = collect("@mark(serial=True, slow=True, group='database')\nclass TestDatabase:\n    @mark(serial=False, slow=False)\n    def test_read(self): pass\n").unwrap();
    let marker = items[0]
        .markers
        .iter()
        .find(|marker| marker.name == "mark")
        .unwrap();
    assert_eq!(
        marker.args.kwargs.get("serial"),
        Some(&MarkerValue::Bool(false))
    );
    assert!(!items[0].is_slow());
    assert_eq!(items[0].groups(), ["database"]);
    assert_eq!(
        items[0]
            .markers
            .iter()
            .filter(|marker| marker.name == "mark")
            .count(),
        1
    );
}

#[test]
fn class_skip_and_method_skip_reason_are_preserved() {
    let items = collect("@skip('class reason')\nclass TestSkipped:\n    def test_inherited_reason(self): assert False\n    @skip('method reason')\n    def test_override_reason(self): assert False\n").unwrap();
    assert!(items.iter().all(TestItem::is_skipped));
    assert_eq!(items[0].skip_reason().as_deref(), Some("class reason"));
    assert_eq!(items[1].skip_reason().as_deref(), Some("method reason"));
}

#[test]
fn marker_defaults_follow_c3_with_per_key_precedence() {
    use taut::markers::MarkerValue;
    let items = collect("@mark(serial=True, slow=True, group='base')\n@skip('base skip')\nclass Base:\n    def test_fail(self): assert False\n@mark(slow=False)\nclass Left(Base): pass\n@mark(group='right')\nclass Right(Base): pass\nclass TestDiamond(Left, Right): pass\n").unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].is_skipped());
    assert_eq!(items[0].skip_reason().as_deref(), Some("base skip"));
    assert!(!items[0].is_slow());
    assert_eq!(items[0].groups(), ["right"]);
    let marker = items[0]
        .markers
        .iter()
        .find(|marker| marker.name == "mark")
        .unwrap();
    assert_eq!(
        marker.args.kwargs.get("serial"),
        Some(&MarkerValue::Bool(true))
    );
}

#[test]
fn stacked_mark_decorators_merge_with_outermost_precedence() {
    use taut::markers::MarkerValue;
    let items = collect("@mark(serial=True)\n@mark(group='outer')\nclass TestStacked:\n    @mark(slow=False)\n    @mark(slow=True, serial=False)\n    def test_one(self): pass\n").unwrap();
    assert!(!items[0].is_slow());
    assert_eq!(items[0].groups(), ["outer"]);
    let marker = items[0]
        .markers
        .iter()
        .find(|marker| marker.name == "mark")
        .unwrap();
    assert_eq!(
        marker.args.kwargs.get("serial"),
        Some(&MarkerValue::Bool(false))
    );
}
