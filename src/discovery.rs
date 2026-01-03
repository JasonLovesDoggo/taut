use anyhow::{Context, Result};
use rustpython_parser::{Parse, ast};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestItem {
    pub file: PathBuf,
    pub function: String,
    pub class: Option<String>,
    pub line: usize,
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
                    items.push(TestItem {
                        file: path.to_path_buf(),
                        function: func.name.to_string(),
                        class: None,
                        line: offset_to_line(&source, func.range.start().into()),
                    });
                }
            }
            ast::Stmt::ClassDef(class) => {
                if class.name.as_str().starts_with("Test") {
                    for body_stmt in &class.body {
                        if let ast::Stmt::FunctionDef(method) = body_stmt {
                            if is_test_name(method.name.as_str()) {
                                items.push(TestItem {
                                    file: path.to_path_buf(),
                                    function: method.name.to_string(),
                                    class: Some(class.name.to_string()),
                                    line: offset_to_line(&source, method.range.start().into()),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(items)
}

/// Extract tests from multiple files, optionally filtering by name
pub fn extract_tests(files: &[PathBuf], filter: Option<&str>) -> Result<Vec<TestItem>> {
    let mut all_items = Vec::new();

    for file in files {
        match extract_tests_from_file(file) {
            Ok(items) => all_items.extend(items),
            Err(e) => eprintln!("Warning: {}", e),
        }
    }

    if let Some(filter) = filter {
        let filter_lower = filter.to_lowercase();
        all_items.retain(|item| {
            item.function.to_lowercase().contains(&filter_lower)
                || item
                    .class
                    .as_ref()
                    .map(|c| c.to_lowercase().contains(&filter_lower))
                    .unwrap_or(false)
        });
    }

    Ok(all_items)
}
