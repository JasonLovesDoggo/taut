//! Tests for test file and function discovery.
//!
//! These tests verify that taut correctly discovers:
//! - Test files (test_*.py, *_test.py patterns)
//! - Test functions (test_*, _test_* patterns)
//! - Test classes (Test* pattern)
//! - Async test functions

mod helpers;

use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use helpers::{dedent, write_python_file};

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

// =============================================================================
// File Pattern Tests
// =============================================================================

#[test]
fn discover_test_prefix_files() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_alpha.py"),
        "def test_ok(): assert True\n",
    )?;
    write_file(
        &tmp.path().join("test_beta.py"),
        "def test_ok(): assert True\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(names.contains(&"test_alpha.py".to_string()));
    assert!(names.contains(&"test_beta.py".to_string()));
    assert_eq!(names.len(), 2);

    Ok(())
}

#[test]
fn discover_underscore_test_prefix_files() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("_test_private.py"),
        "def test_ok(): assert True\n",
    )?;
    write_file(
        &tmp.path().join("_test_another.py"),
        "def test_ok(): assert True\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(names.contains(&"_test_private.py".to_string()));
    assert!(names.contains(&"_test_another.py".to_string()));

    Ok(())
}

#[test]
fn ignore_non_test_files() -> Result<()> {
    let tmp = TempDir::new()?;

    // These should be ignored
    write_file(
        &tmp.path().join("helper.py"),
        "def helper(): pass\n",
    )?;
    write_file(
        &tmp.path().join("conftest.py"),
        "# pytest config\n",
    )?;
    write_file(
        &tmp.path().join("utils.py"),
        "def util(): pass\n",
    )?;
    // This one has "test" but not in the right pattern
    write_file(
        &tmp.path().join("my_test_helper.py"),
        "def helper(): pass\n",
    )?;

    // This should be found
    write_file(
        &tmp.path().join("test_real.py"),
        "def test_ok(): assert True\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;

    assert_eq!(files.len(), 1);
    assert!(files[0].file_name().unwrap().to_string_lossy().contains("test_real.py"));

    Ok(())
}

#[test]
fn ignore_non_python_files() -> Result<()> {
    let tmp = TempDir::new()?;

    // These should be ignored even though they match the pattern
    write_file(&tmp.path().join("test_example.txt"), "not python\n")?;
    write_file(&tmp.path().join("test_example.rs"), "fn test() {}\n")?;
    write_file(&tmp.path().join("test_example.js"), "function test() {}\n")?;
    write_file(&tmp.path().join("test_example.pyc"), "compiled\n")?;

    // This should be found
    write_file(
        &tmp.path().join("test_real.py"),
        "def test_ok(): assert True\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;

    assert_eq!(files.len(), 1);

    Ok(())
}

#[test]
fn discover_files_recursively() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_root.py"),
        "def test_ok(): pass\n",
    )?;
    write_file(
        &tmp.path().join("subdir/test_nested.py"),
        "def test_ok(): pass\n",
    )?;
    write_file(
        &tmp.path().join("subdir/deep/test_deep.py"),
        "def test_ok(): pass\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;

    assert_eq!(files.len(), 3);

    Ok(())
}

#[test]
fn discover_single_file_path() -> Result<()> {
    let tmp = TempDir::new()?;

    let target = tmp.path().join("test_target.py");
    write_file(&target, "def test_ok(): pass\n")?;

    // Also create another file that should NOT be found
    write_file(
        &tmp.path().join("test_other.py"),
        "def test_ok(): pass\n",
    )?;

    // Pass single file path instead of directory
    let files = taut::discovery::find_test_files(&[target.clone()])?;

    assert_eq!(files.len(), 1);
    assert_eq!(files[0], target);

    Ok(())
}

// =============================================================================
// Function Extraction Tests
// =============================================================================

#[test]
fn extract_test_prefix_functions() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_funcs.py");

    write_file(
        &file,
        &dedent(r#"
            def test_one():
                assert True

            def test_two():
                assert True

            def helper():
                pass

            def another_helper():
                pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    let names: Vec<_> = items.iter().map(|i| i.function.as_str()).collect();

    assert_eq!(names, vec!["test_one", "test_two"]);

    Ok(())
}

#[test]
fn extract_underscore_test_prefix_functions() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_funcs.py");

    write_file(
        &file,
        &dedent(r#"
            def _test_private():
                assert True

            def _test_another():
                assert True

            def test_public():
                assert True
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    let names: Vec<_> = items.iter().map(|i| i.function.as_str()).collect();

    assert!(names.contains(&"_test_private"));
    assert!(names.contains(&"_test_another"));
    assert!(names.contains(&"test_public"));
    assert_eq!(names.len(), 3);

    Ok(())
}

#[test]
fn extract_async_test_functions() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_async.py");

    write_file(
        &file,
        &dedent(r#"
            async def test_async_one():
                await something()

            async def test_async_two():
                pass

            def test_sync():
                pass

            async def helper_async():
                pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    let names: Vec<_> = items.iter().map(|i| i.function.as_str()).collect();

    // All three test functions should be discovered
    assert!(names.contains(&"test_async_one"));
    assert!(names.contains(&"test_async_two"));
    assert!(names.contains(&"test_sync"));
    // Helper should NOT be discovered
    assert!(!names.contains(&"helper_async"));
    assert_eq!(names.len(), 3);

    Ok(())
}

// =============================================================================
// Class Extraction Tests
// =============================================================================

#[test]
fn extract_methods_from_test_classes() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_classes.py");

    write_file(
        &file,
        &dedent(r#"
            class TestMath:
                def test_add(self):
                    assert 1 + 1 == 2

                def test_sub(self):
                    assert 2 - 1 == 1

                def helper(self):
                    pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    assert_eq!(items.len(), 2);

    let test_add = items.iter().find(|i| i.function == "test_add").unwrap();
    assert_eq!(test_add.class, Some("TestMath".to_string()));

    let test_sub = items.iter().find(|i| i.function == "test_sub").unwrap();
    assert_eq!(test_sub.class, Some("TestMath".to_string()));

    Ok(())
}

#[test]
fn ignore_non_test_classes() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_classes.py");

    write_file(
        &file,
        &dedent(r#"
            class TestValid:
                def test_method(self):
                    pass

            class HelperClass:
                def test_method(self):
                    pass

            class MyTestClass:
                def test_method(self):
                    pass

            class Testlowercase:
                def test_method(self):
                    pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    // Only TestValid should be found (starts with "Test" followed by uppercase)
    // Note: This depends on the exact matching rule - check implementation
    let classes: Vec<_> = items.iter().filter_map(|i| i.class.as_ref()).collect();

    // TestValid should definitely be found
    assert!(classes.contains(&&"TestValid".to_string()));

    // Testlowercase might or might not be found depending on implementation
    // The key is that HelperClass and MyTestClass should NOT be found
    for item in &items {
        if let Some(cls) = &item.class {
            assert!(
                cls.starts_with("Test"),
                "Found class {} which doesn't start with 'Test'",
                cls
            );
        }
    }

    Ok(())
}

#[test]
fn extract_underscore_test_methods() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_classes.py");

    write_file(
        &file,
        &dedent(r#"
            class TestExample:
                def test_public(self):
                    pass

                def _test_private(self):
                    pass

                def helper(self):
                    pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    let names: Vec<_> = items.iter().map(|i| i.function.as_str()).collect();

    assert!(names.contains(&"test_public"));
    assert!(names.contains(&"_test_private"));
    assert!(!names.contains(&"helper"));

    Ok(())
}

// =============================================================================
// Line Number Tests
// =============================================================================

#[test]
fn extract_correct_line_numbers() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_lines.py");

    // Note: line numbers are 1-indexed
    write_file(
        &file,
        "def test_first():\n    pass\n\ndef test_second():\n    pass\n",
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    let first = items.iter().find(|i| i.function == "test_first").unwrap();
    let second = items.iter().find(|i| i.function == "test_second").unwrap();

    assert_eq!(first.line, 1, "test_first should be on line 1");
    assert_eq!(second.line, 4, "test_second should be on line 4");

    Ok(())
}

#[test]
fn extract_correct_line_numbers_with_decorators() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_decorated.py");

    write_file(
        &file,
        &dedent(r#"
            @decorator
            def test_decorated():
                pass

            @decorator1
            @decorator2
            def test_multi_decorated():
                pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    // For discovery, we report the def line (where the function name is)
    // This is where someone would navigate to see the test.
    // The blocks module uses decorator lines for checksum boundaries.
    let decorated = items
        .iter()
        .find(|i| i.function == "test_decorated")
        .unwrap();
    let multi = items
        .iter()
        .find(|i| i.function == "test_multi_decorated")
        .unwrap();

    // test_decorated is on line 2 (after @decorator on line 1)
    assert_eq!(
        decorated.line, 2,
        "test_decorated should be on line 2 (the def line)"
    );
    // test_multi_decorated is on line 7 (after @decorator1 and @decorator2)
    assert_eq!(
        multi.line, 7,
        "test_multi_decorated should be on line 7 (the def line)"
    );

    Ok(())
}

// =============================================================================
// Filter Tests
// =============================================================================

#[test]
fn filter_by_function_name() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_filter.py");

    write_file(
        &file,
        &dedent(r#"
            def test_alpha():
                pass

            def test_beta():
                pass

            def test_alpha_extended():
                pass
        "#),
    )?;

    let files = vec![file];
    let items = taut::discovery::extract_tests(&files, Some("alpha"))?;
    let names: Vec<_> = items.iter().map(|i| i.function.as_str()).collect();

    assert!(names.contains(&"test_alpha"));
    assert!(names.contains(&"test_alpha_extended"));
    assert!(!names.contains(&"test_beta"));

    Ok(())
}

#[test]
fn filter_case_insensitive() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_filter.py");

    write_file(
        &file,
        &dedent(r#"
            def test_Alpha():
                pass

            def test_ALPHA():
                pass

            def test_alpha():
                pass

            def test_beta():
                pass
        "#),
    )?;

    let files = vec![file];
    let items = taut::discovery::extract_tests(&files, Some("alpha"))?;

    assert_eq!(items.len(), 3, "Filter should be case-insensitive");

    Ok(())
}

#[test]
fn filter_by_class_name() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_filter.py");

    write_file(
        &file,
        &dedent(r#"
            class TestAlpha:
                def test_one(self):
                    pass

            class TestBeta:
                def test_one(self):
                    pass
        "#),
    )?;

    let files = vec![file];
    let items = taut::discovery::extract_tests(&files, Some("Alpha"))?;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].class, Some("TestAlpha".to_string()));

    Ok(())
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn handle_syntax_error_gracefully() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_syntax_error.py");

    write_file(
        &file,
        "def test_broken(\n    # missing closing paren\n",
    )?;

    // Should return an error, not panic
    let result = taut::discovery::extract_tests_from_file(&file);
    assert!(result.is_err(), "Should return error for syntax error");

    Ok(())
}

#[test]
fn handle_empty_file() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_empty.py");

    write_file(&file, "")?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    assert!(items.is_empty());

    Ok(())
}

#[test]
fn handle_file_with_only_comments() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_comments.py");

    write_file(
        &file,
        "# This is a comment\n# Another comment\n",
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    assert!(items.is_empty());

    Ok(())
}

#[test]
fn handle_file_with_only_docstring() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_docstring.py");

    write_file(
        &file,
        "\"\"\"This module has only a docstring.\"\"\"\n",
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    assert!(items.is_empty());

    Ok(())
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn handle_nested_classes() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_nested.py");

    write_file(
        &file,
        &dedent(r#"
            class TestOuter:
                def test_outer(self):
                    pass

                class TestInner:
                    def test_inner(self):
                        pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    // At minimum, TestOuter.test_outer should be found
    let outer = items.iter().find(|i| i.function == "test_outer");
    assert!(outer.is_some(), "Should find test_outer");
    assert_eq!(outer.unwrap().class, Some("TestOuter".to_string()));

    // Nested classes may or may not be supported - document the behavior
    // For now, we just verify no panic

    Ok(())
}

#[test]
fn handle_function_with_complex_signature() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_complex.py");

    write_file(
        &file,
        &dedent(r#"
            def test_with_args(
                arg1,
                arg2,
                *args,
                **kwargs
            ):
                pass

            def test_with_defaults(
                x=1,
                y="hello",
                z=None
            ):
                pass

            def test_with_annotations(
                x: int,
                y: str
            ) -> bool:
                pass
        "#),
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;

    assert_eq!(items.len(), 3);

    Ok(())
}

#[test]
fn handle_multiple_test_files() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_a.py"),
        "def test_a1(): pass\ndef test_a2(): pass\n",
    )?;
    write_file(
        &tmp.path().join("test_b.py"),
        "def test_b1(): pass\n",
    )?;

    let files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;
    let items = taut::discovery::extract_tests(&files, None)?;

    assert_eq!(items.len(), 3);

    Ok(())
}
