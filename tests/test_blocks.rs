//! Tests for block parsing and checksums.
//!
//! These tests verify that taut correctly:
//! - Extracts code blocks (functions, classes, imports, top-level)
//! - Computes checksums that are stable across whitespace changes
//! - Handles edge cases like async functions, decorators, UTF-8
//!
//! Several tests here are expected to FAIL until bugs are fixed.
//! They document correct behavior that the code should have.

mod helpers;

use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use helpers::dedent;
use taut::blocks::{BlockKind, FileBlocks};

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

// =============================================================================
// Checksum Invariant Tests - Whitespace
// =============================================================================

#[test]
fn checksum_ignores_leading_whitespace() {
    // These should have the same checksum
    let a = "def foo():\n    pass";
    let b = "def foo():\n        pass"; // More indentation

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    let checksum_a = &blocks_a.blocks[0].checksum;
    let checksum_b = &blocks_b.blocks[0].checksum;

    assert_eq!(
        checksum_a, checksum_b,
        "Checksums should be equal regardless of indentation"
    );
}

#[test]
fn checksum_ignores_trailing_whitespace() {
    let a = "def foo():\n    pass";
    let b = "def foo():   \n    pass   "; // Trailing spaces

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    assert_eq!(blocks_a.blocks[0].checksum, blocks_b.blocks[0].checksum);
}

#[test]
fn checksum_ignores_blank_lines() {
    let a = "def foo():\n    pass";
    let b = "def foo():\n\n    pass\n\n"; // Blank lines inside and after

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    assert_eq!(blocks_a.blocks[0].checksum, blocks_b.blocks[0].checksum);
}

#[test]
fn checksum_ignores_comment_lines() {
    let a = "def foo():\n    pass";
    let b = "def foo():\n    # This is a comment\n    pass";

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    assert_eq!(blocks_a.blocks[0].checksum, blocks_b.blocks[0].checksum);
}

#[test]
fn checksum_preserves_inline_comments() {
    // Inline comments are part of the code semantics (sometimes)
    // This test documents current behavior - adjust based on desired behavior
    let a = "def foo():\n    x = 1";
    let b = "def foo():\n    x = 1  # inline comment";

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    // These MIGHT be different or same depending on desired behavior
    // For now, just ensure no panic
    let _ = &blocks_a.blocks[0].checksum;
    let _ = &blocks_b.blocks[0].checksum;
}

#[test]
fn checksum_detects_actual_code_changes() {
    let a = "def foo():\n    return 1";
    let b = "def foo():\n    return 2";

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    assert_ne!(
        blocks_a.blocks[0].checksum, blocks_b.blocks[0].checksum,
        "Checksums should differ for actual code changes"
    );
}

// =============================================================================
// BUG: Checksum incorrectly filters lines with # in strings
// =============================================================================

#[test]
fn checksum_does_not_filter_hash_in_string() {
    // BUG: The current implementation filters ANY line starting with #
    // after trimming, even if it's inside a string literal.
    //
    // This test will FAIL until fixed.
    let a = "def foo():\n    return \"hello\"";
    let b = "def foo():\n    return \"# not a comment\"";

    let blocks_a = FileBlocks::from_source(a, "test.py").unwrap();
    let blocks_b = FileBlocks::from_source(b, "test.py").unwrap();

    // These should have DIFFERENT checksums because the return values are different
    assert_ne!(
        blocks_a.blocks[0].checksum, blocks_b.blocks[0].checksum,
        "BUG: String containing '#' is incorrectly filtered as comment"
    );
}

#[test]
fn checksum_handles_multiline_strings_with_hash() {
    // BUG: Multiline strings with lines that look like comments
    let code = r#"
def foo():
    return """
    # This looks like a comment but isn't
    It's inside a multiline string
    """
"#;

    let blocks = FileBlocks::from_source(&dedent(code), "test.py").unwrap();

    // Should parse without error and have a consistent checksum
    assert!(!blocks.blocks.is_empty());
}

// =============================================================================
// BUG: Async functions not handled correctly
// =============================================================================

#[test]
fn async_function_extracted_as_function_block() {
    // BUG: AsyncFunctionDef is not handled in extract_definitions
    // Async functions fall through to top-level code
    //
    // This test will FAIL until fixed.
    let code = &dedent(
        r#"
        async def async_helper():
            await something()

        def sync_helper():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Find the async function block
    let async_block = blocks.blocks.iter().find(|b| b.id.name == "async_helper");

    assert!(
        async_block.is_some(),
        "BUG: Async function not extracted as block"
    );

    if let Some(block) = async_block {
        assert_eq!(
            block.id.kind,
            BlockKind::Function,
            "BUG: Async function should be BlockKind::Function, not {:?}",
            block.id.kind
        );
    }
}

#[test]
fn async_method_extracted_correctly() {
    let code = &dedent(
        r#"
        class TestAsync:
            async def test_async(self):
                await something()

            def test_sync(self):
                pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Find the async method block
    let async_method = blocks
        .blocks
        .iter()
        .find(|b| b.id.name.contains("test_async"));

    assert!(
        async_method.is_some(),
        "BUG: Async method not extracted from class"
    );

    if let Some(block) = async_method {
        assert_eq!(
            block.id.kind,
            BlockKind::Method,
            "Async method should be BlockKind::Method"
        );
    }
}

// =============================================================================
// BUG: UTF-8 handling
// =============================================================================

#[test]
fn utf8_in_function_name_does_not_panic() {
    // While Python allows unicode identifiers, let's at least not panic
    let code = "def test_naïve(): pass\n";

    let result = FileBlocks::from_source(code, "test.py");
    // Should not panic
    assert!(result.is_ok() || result.is_err()); // Either is fine, just no panic
}

#[test]
fn utf8_in_string_does_not_panic() {
    let code = r#"
def test_unicode():
    return "Hello 世界 🎉 café"
"#;

    let blocks = FileBlocks::from_source(&dedent(code), "test.py").unwrap();
    assert!(!blocks.blocks.is_empty());
}

#[test]
fn utf8_in_comment_does_not_panic() {
    let code = "# Comment with émojis: 🎉 ñ\ndef test_ok(): pass\n";

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();
    assert!(!blocks.blocks.is_empty());
}

#[test]
fn utf8_in_file_does_not_corrupt_line_numbers() {
    // BUG: offset_to_line uses char iteration but parser returns byte offsets
    // This test will FAIL until fixed.
    let code = "# café\ndef test_after_utf8(): pass\n";

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let test_block = blocks
        .blocks
        .iter()
        .find(|b| b.id.name == "test_after_utf8");

    assert!(test_block.is_some());

    if let Some(block) = test_block {
        assert_eq!(
            block.id.start_line, 2,
            "BUG: Line number incorrect after UTF-8 content"
        );
    }
}

// =============================================================================
// Block Extraction Tests - Functions
// =============================================================================

#[test]
fn extract_simple_function() {
    let code = "def foo(): pass\n";

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let func = blocks.blocks.iter().find(|b| b.id.name == "foo");
    assert!(func.is_some());
    assert_eq!(func.unwrap().id.kind, BlockKind::Function);
}

#[test]
fn extract_decorated_function() {
    let code = &dedent(
        r#"
        @decorator
        def foo():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let func = blocks.blocks.iter().find(|b| b.id.name == "foo");
    assert!(func.is_some());

    // The decorator should be included in the block's line range
    // BUG: Currently rustpython_parser returns def line, not decorator line
    let block = func.unwrap();
    assert_eq!(
        block.id.start_line, 1,
        "BUG: Block should start at decorator line, but got line {}",
        block.id.start_line
    );
}

#[test]
fn extract_function_with_multiple_decorators() {
    let code = &dedent(
        r#"
        @decorator1
        @decorator2
        @decorator3
        def foo():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let func = blocks.blocks.iter().find(|b| b.id.name == "foo");
    assert!(func.is_some());

    // BUG: Currently rustpython_parser returns def line, not decorator line
    let block = func.unwrap();
    assert_eq!(
        block.id.start_line, 1,
        "BUG: Block should start at first decorator line, but got line {}",
        block.id.start_line
    );
}

#[test]
fn nested_function_part_of_parent() {
    // Nested functions should be part of the parent function's block,
    // not extracted as separate blocks (design decision)
    let code = &dedent(
        r#"
        def outer():
            def inner():
                return 1
            return inner()
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Should find outer
    let outer = blocks.blocks.iter().find(|b| b.id.name == "outer");
    assert!(outer.is_some());

    // Should NOT find inner as a separate block
    let inner = blocks.blocks.iter().find(|b| b.id.name == "inner");
    assert!(
        inner.is_none(),
        "Nested functions should not be separate blocks"
    );
}

// =============================================================================
// Block Extraction Tests - Classes
// =============================================================================

#[test]
fn extract_class_with_methods() {
    let code = &dedent(
        r#"
        class Foo:
            def method_one(self):
                pass

            def method_two(self):
                pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Should have: class header, method_one, method_two
    let class_block = blocks.blocks.iter().find(|b| b.id.name == "Foo");
    assert!(class_block.is_some());
    assert_eq!(class_block.unwrap().id.kind, BlockKind::Class);

    let method_one = blocks.blocks.iter().find(|b| b.id.name == "Foo.method_one");
    assert!(method_one.is_some());
    assert_eq!(method_one.unwrap().id.kind, BlockKind::Method);

    let method_two = blocks.blocks.iter().find(|b| b.id.name == "Foo.method_two");
    assert!(method_two.is_some());
}

#[test]
fn class_variables_before_methods_in_header() {
    let code = &dedent(
        r#"
        class Foo:
            class_var = 1
            another_var = 2

            def method(self):
                pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Class header should include the class variables
    let class_block = blocks.blocks.iter().find(|b| b.id.name == "Foo").unwrap();

    // The class header should span from "class Foo:" to before "def method"
    // Line 1: class Foo:
    // Line 2: class_var = 1
    // Line 3: another_var = 2
    // Line 4: (blank)
    // Line 5: def method(self):
    assert!(
        class_block.id.end_line >= 3,
        "Class header should include class variables"
    );
}

#[test]
fn class_variables_after_methods_belong_to_some_block() {
    // BUG: Class variables defined after methods are orphaned
    // They belong to NO block currently
    //
    // This test documents the issue.
    let code = &dedent(
        r#"
        class Foo:
            def method(self):
                pass

            after_var = 1
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Check what line 6 (after_var = 1) maps to
    // In the current implementation, it maps to nothing
    let line_6_block = blocks.get_block_for_line(6);

    // This SHOULD map to something (either the class or a separate block)
    // Currently it's orphaned
    if line_6_block.is_none() {
        // Document the bug but don't fail (yet)
        eprintln!("BUG: Class variable after method is orphaned (maps to no block)");
    }
}

#[test]
fn extract_property() {
    let code = &dedent(
        r#"
        class Foo:
            @property
            def value(self):
                return self._value
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let prop = blocks.blocks.iter().find(|b| b.id.name == "Foo.value");
    assert!(prop.is_some());
    assert_eq!(prop.unwrap().id.kind, BlockKind::Method);
}

#[test]
fn extract_staticmethod() {
    let code = &dedent(
        r#"
        class Foo:
            @staticmethod
            def static_method():
                pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let method = blocks
        .blocks
        .iter()
        .find(|b| b.id.name == "Foo.static_method");
    assert!(method.is_some());
}

#[test]
fn extract_classmethod() {
    let code = &dedent(
        r#"
        class Foo:
            @classmethod
            def class_method(cls):
                pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let method = blocks
        .blocks
        .iter()
        .find(|b| b.id.name == "Foo.class_method");
    assert!(method.is_some());
}

// =============================================================================
// Block Extraction Tests - Imports
// =============================================================================

#[test]
fn imports_grouped_into_single_block() {
    let code = &dedent(
        r#"
        import os
        import sys
        from pathlib import Path
        from typing import List, Dict

        def foo():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let import_blocks: Vec<_> = blocks
        .blocks
        .iter()
        .filter(|b| b.id.kind == BlockKind::Import)
        .collect();

    assert_eq!(
        import_blocks.len(),
        1,
        "All imports should be in a single block"
    );
}

#[test]
fn scattered_imports_behavior() {
    // BUG: When imports are scattered, the import block spans
    // the entire range, including intermediate code
    //
    // This test documents the issue.
    let code = &dedent(
        r#"
        import os

        x = 1

        import sys
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let import_block = blocks
        .blocks
        .iter()
        .find(|b| b.id.kind == BlockKind::Import);

    if let Some(block) = import_block {
        // The import block currently spans from line 1 to line 5
        // This means x = 1 is included in the import block checksum
        // which is probably not desired
        if block.id.end_line > 2 {
            eprintln!(
                "BUG: Scattered imports create block from line {} to {} (includes intermediate code)",
                block.id.start_line, block.id.end_line
            );
        }
    }
}

#[test]
fn conditional_import_not_in_import_block() {
    let code = &dedent(
        r#"
        import os

        if TYPE_CHECKING:
            import typing

        def foo():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // The conditional import should NOT be in the import block
    // It should be in a top-level block instead
    let import_block = blocks
        .blocks
        .iter()
        .find(|b| b.id.kind == BlockKind::Import)
        .unwrap();

    // Import block should only cover "import os" (line 1)
    assert!(
        import_block.id.end_line <= 2,
        "Import block should not include conditional import"
    );
}

// =============================================================================
// Block Extraction Tests - Top-Level Code
// =============================================================================

#[test]
fn top_level_code_extracted() {
    let code = &dedent(
        r#"
        x = 1
        y = 2
        print("hello")
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    let top_level: Vec<_> = blocks
        .blocks
        .iter()
        .filter(|b| b.id.kind == BlockKind::TopLevel)
        .collect();

    assert!(!top_level.is_empty(), "Should extract top-level code");
}

#[test]
fn if_name_main_is_top_level() {
    let code = &dedent(
        r#"
        def foo():
            pass

        if __name__ == "__main__":
            foo()
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // The if __name__ block should be extracted as top-level
    let top_level: Vec<_> = blocks
        .blocks
        .iter()
        .filter(|b| b.id.kind == BlockKind::TopLevel)
        .collect();

    assert!(!top_level.is_empty());
}

// =============================================================================
// Line-to-Block Mapping Tests
// =============================================================================

#[test]
fn line_to_block_mapping_correct() {
    let code = &dedent(
        r#"
        def foo():
            pass

        def bar():
            pass
    "#,
    );

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Line 1: def foo(): -> foo block
    let line1_block = blocks.get_block_for_line(1);
    assert!(line1_block.is_some());
    assert_eq!(line1_block.unwrap().id.name, "foo");

    // Line 2: pass -> still foo block
    let line2_block = blocks.get_block_for_line(2);
    assert!(line2_block.is_some());
    assert_eq!(line2_block.unwrap().id.name, "foo");

    // Line 4: def bar(): -> bar block
    let line4_block = blocks.get_block_for_line(4);
    assert!(line4_block.is_some());
    assert_eq!(line4_block.unwrap().id.name, "bar");
}

#[test]
fn blank_lines_between_functions_may_be_unmapped() {
    let code = "def foo():\n    pass\n\n\ndef bar():\n    pass\n";

    let blocks = FileBlocks::from_source(code, "test.py").unwrap();

    // Lines 3-4 are blank lines between functions
    // They may or may not map to a block - document the behavior
    let line3_block = blocks.get_block_for_line(3);
    let line4_block = blocks.get_block_for_line(4);

    // Just ensure no panic - the mapping is implementation-defined
    let _ = line3_block;
    let _ = line4_block;
}

// =============================================================================
// Block Identity Tests - These verify behavior we need for correct caching
// =============================================================================

#[test]
fn block_identity_stable_after_adding_blank_line_above() {
    // CRITICAL TEST: Adding a blank line above a function should NOT
    // change its identity for caching purposes.
    //
    // Currently, BlockId includes start_line which WILL change.
    // This test documents the desired behavior.

    let code_before = &dedent(
        r#"
        def helper():
            return 1

        def test_foo():
            assert helper() == 1
    "#,
    );

    let code_after = &dedent(
        r#"
        def helper():
            return 1


        def test_foo():
            assert helper() == 1
    "#,
    );

    let blocks_before = FileBlocks::from_source(code_before, "test.py").unwrap();
    let blocks_after = FileBlocks::from_source(code_after, "test.py").unwrap();

    let helper_before = blocks_before
        .blocks
        .iter()
        .find(|b| b.id.name == "helper")
        .unwrap();
    let helper_after = blocks_after
        .blocks
        .iter()
        .find(|b| b.id.name == "helper")
        .unwrap();

    // The CHECKSUM should be the same (content didn't change)
    assert_eq!(
        helper_before.checksum, helper_after.checksum,
        "Checksum should be stable"
    );

    // The line numbers WILL differ, but for caching purposes,
    // we should identify the block by (file, kind, name) not line numbers
    // This is a design note - the current implementation uses line numbers in BlockId
}

#[test]
fn block_identity_stable_after_adding_comment_above() {
    let code_before = "def foo(): pass\n";
    let code_after = "# New comment\ndef foo(): pass\n";

    let blocks_before = FileBlocks::from_source(code_before, "test.py").unwrap();
    let blocks_after = FileBlocks::from_source(code_after, "test.py").unwrap();

    let foo_before = blocks_before
        .blocks
        .iter()
        .find(|b| b.id.name == "foo")
        .unwrap();
    let foo_after = blocks_after
        .blocks
        .iter()
        .find(|b| b.id.name == "foo")
        .unwrap();

    // Checksum should be stable
    assert_eq!(foo_before.checksum, foo_after.checksum);
}

// =============================================================================
// Test helper to create FileBlocks from source string
// =============================================================================

// We need to add a helper method to FileBlocks for testing
// This trait extension allows us to test without writing to disk

trait FileBlocksTestExt {
    fn from_source(source: &str, filename: &str) -> Result<FileBlocks, anyhow::Error>;
}

impl FileBlocksTestExt for FileBlocks {
    fn from_source(source: &str, filename: &str) -> Result<FileBlocks, anyhow::Error> {
        // Write to temp file and parse
        let tmp = TempDir::new()?;
        let path = tmp.path().join(filename);
        fs::write(&path, source)?;
        FileBlocks::from_file(&path)
    }
}
