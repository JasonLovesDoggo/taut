use anyhow::{Context, Result};
use rustpython_parser::{Parse, ast};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::filter::TestFilter;
use crate::markers::{self, Marker};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestItem {
    pub file: PathBuf,
    pub function: String,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub line: usize,
    /// Markers attached to this test (@skip, @mark, @parallel)
    #[serde(default)]
    pub markers: Vec<Marker>,
}

impl TestItem {
    /// Returns a unique identifier for this test (e.g., "tests/test_example.py::TestMath::test_add")
    pub fn id(&self) -> String {
        let file = self.file.display();
        match &self.class {
            Some(class) => format!("{}::{}::{}", file, class, self.function),
            None => format!("{}::{}", file, self.function),
        }
    }

    /// Check if this test has the @skip marker.
    pub fn is_skipped(&self) -> bool {
        markers::is_skipped(&self.markers)
    }

    /// Get the skip reason if present.
    pub fn skip_reason(&self) -> Option<String> {
        markers::get_skip_reason(&self.markers)
    }

    /// Check if this test has the @parallel marker.
    pub fn is_parallel(&self) -> bool {
        markers::is_parallel(&self.markers)
    }

    /// Check if this test has @mark(slow=True).
    pub fn is_slow(&self) -> bool {
        markers::is_slow(&self.markers)
    }

    /// Get the group(s) from @mark(group="...").
    pub fn groups(&self) -> Vec<String> {
        markers::get_groups(&self.markers)
    }
}

/// Find all Python test files in the given paths.
///
/// A file is considered a test file if its name matches either:
/// - `test_*.py`
/// - `*_test*.py`
pub fn find_test_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut test_files = Vec::new();

    for path in paths {
        if path.is_file() {
            if is_test_file(path) {
                test_files.push(path.clone());
            }
        } else if path.is_dir() {
            for entry in WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let p = entry.path();
                if is_test_file(p) {
                    test_files.push(p.to_path_buf());
                }
            }
        }
    }

    test_files.sort();
    Ok(test_files)
}

fn is_test_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    if !file_name.ends_with(".py") {
        return false;
    }

    file_name.starts_with("test_") || file_name.starts_with("_test")
}

fn is_test_name(name: &str) -> bool {
    name.starts_with("test_") || name.starts_with("_test")
}

/// Convert byte offset to line number (1-indexed)
fn offset_to_line(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

/// Parse a Python file and extract test items
pub fn extract_tests_from_file(path: &Path) -> Result<Vec<TestItem>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let ast = ast::Suite::parse(&source, "<test>")
        .map_err(|e| anyhow::anyhow!("Parse error in {}: {}", path.display(), e))?;

    let mut items = Vec::new();

    for stmt in ast {
        match stmt {
            ast::Stmt::FunctionDef(func) => {
                if is_test_name(func.name.as_str()) {
                    let func_markers = markers::extract_markers(&func.decorator_list);
                    items.push(TestItem {
                        file: path.to_path_buf(),
                        function: func.name.to_string(),
                        class: None,
                        line: offset_to_line(&source, func.range.start().into()),
                        markers: func_markers,
                    });
                }
            }
            ast::Stmt::AsyncFunctionDef(func) => {
                if is_test_name(func.name.as_str()) {
                    let func_markers = markers::extract_markers(&func.decorator_list);
                    items.push(TestItem {
                        file: path.to_path_buf(),
                        function: func.name.to_string(),
                        class: None,
                        line: offset_to_line(&source, func.range.start().into()),
                        markers: func_markers,
                    });
                }
            }
            ast::Stmt::ClassDef(class) => {
                if class.name.as_str().starts_with("Test") {
                    // Extract class-level markers (e.g., @parallel on class)
                    let class_markers = markers::extract_class_markers(&class.decorator_list);

                    for body_stmt in &class.body {
                        match body_stmt {
                            ast::Stmt::FunctionDef(method) => {
                                if is_test_name(method.name.as_str()) {
                                    // Combine class markers with method markers
                                    let mut method_markers =
                                        markers::extract_markers(&method.decorator_list);
                                    // Class @parallel applies to all methods
                                    for class_marker in &class_markers {
                                        if !method_markers
                                            .iter()
                                            .any(|m| m.name == class_marker.name)
                                        {
                                            method_markers.push(class_marker.clone());
                                        }
                                    }
                                    items.push(TestItem {
                                        file: path.to_path_buf(),
                                        function: method.name.to_string(),
                                        class: Some(class.name.to_string()),
                                        line: offset_to_line(&source, method.range.start().into()),
                                        markers: method_markers,
                                    });
                                }
                            }
                            ast::Stmt::AsyncFunctionDef(method) => {
                                if is_test_name(method.name.as_str()) {
                                    let mut method_markers =
                                        markers::extract_markers(&method.decorator_list);
                                    for class_marker in &class_markers {
                                        if !method_markers
                                            .iter()
                                            .any(|m| m.name == class_marker.name)
                                        {
                                            method_markers.push(class_marker.clone());
                                        }
                                    }
                                    items.push(TestItem {
                                        file: path.to_path_buf(),
                                        function: method.name.to_string(),
                                        class: Some(class.name.to_string()),
                                        line: offset_to_line(&source, method.range.start().into()),
                                        markers: method_markers,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(items)
}

/// Extract tests from multiple files, optionally filtering by glob pattern.
///
/// Filter patterns (Go-style):
/// - `test_user` - matches any test containing "test_user"
/// - `test_*login` - glob pattern with wildcard
/// - `TestClass/*` - matches all methods in TestClass (/ means ::)
/// - `file.py::test_foo` - file-specific filtering
pub fn extract_tests(files: &[PathBuf], filter_pattern: Option<&str>) -> Result<Vec<TestItem>> {
    let mut all_items = Vec::new();

    for file in files {
        match extract_tests_from_file(file) {
            Ok(items) => all_items.extend(items),
            Err(e) => eprintln!("Warning: {}", e),
        }
    }

    // Apply glob-based filter if provided
    if let Some(pattern) = filter_pattern {
        if !pattern.is_empty() {
            let test_filter = TestFilter::new(pattern)
                .map_err(|e| anyhow::anyhow!("Invalid filter pattern '{}': {}", pattern, e))?;
            all_items.retain(|item| test_filter.matches(&item.id()));
        }
    }

    Ok(all_items)
}
