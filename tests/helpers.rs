//! Test helpers for TAUT tests.
//!
//! Provides utilities similar to pytest's `pytester` fixture for creating
//! temporary test projects, running taut, and asserting on results.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result};
use tempfile::TempDir;

/// A temporary test project for integration tests.
///
/// Similar to pytest's `pytester` fixture - creates a temporary directory
/// with Python test files and provides methods to run taut and assert results.
pub struct TempProject {
    pub dir: TempDir,
    pub files: HashMap<String, String>,
}

impl TempProject {
    /// Create a new empty temporary project.
    pub fn new() -> Result<Self> {
        let dir = TempDir::new().context("Failed to create temp directory")?;
        Ok(Self {
            dir,
            files: HashMap::new(),
        })
    }

    /// Get the path to the temporary directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write a Python file to the project.
    ///
    /// # Example
    /// ```ignore
    /// project.write_file("test_example.py", r#"
    /// def test_pass():
    ///     assert True
    /// "#)?;
    /// ```
    pub fn write_file(&mut self, name: &str, content: &str) -> Result<PathBuf> {
        let path = self.dir.path().join(name);

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Dedent the content (remove common leading whitespace)
        let dedented = dedent(content);

        fs::write(&path, &dedented)
            .with_context(|| format!("Failed to write file: {}", path.display()))?;

        self.files.insert(name.to_string(), dedented);
        Ok(path)
    }

    /// Write multiple Python files at once.
    pub fn write_files(&mut self, files: &[(&str, &str)]) -> Result<Vec<PathBuf>> {
        files
            .iter()
            .map(|(name, content)| self.write_file(name, content))
            .collect()
    }

    /// Create a subdirectory in the project.
    pub fn mkdir(&self, name: &str) -> Result<PathBuf> {
        let path = self.dir.path().join(name);
        fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
        Ok(path)
    }

    /// Read a file from the project.
    pub fn read_file(&self, name: &str) -> Result<String> {
        let path = self.dir.path().join(name);
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file: {}", path.display()))
    }

    /// Check if a file exists in the project.
    pub fn file_exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    /// Get the absolute path to a file in the project.
    pub fn file_path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

/// Result of running taut on a project.
#[derive(Debug)]
pub struct TautResult {
    pub output: Output,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl TautResult {
    /// Check if taut exited successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Assert that taut exited successfully.
    pub fn assert_success(&self) {
        assert!(
            self.success(),
            "Expected success but got exit code {}.\nstdout:\n{}\nstderr:\n{}",
            self.exit_code,
            self.stdout,
            self.stderr
        );
    }

    /// Assert that taut failed (non-zero exit code).
    pub fn assert_failure(&self) {
        assert!(
            !self.success(),
            "Expected failure but got success.\nstdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
    }

    /// Assert that stdout contains the given substring.
    pub fn assert_stdout_contains(&self, expected: &str) {
        assert!(
            self.stdout.contains(expected),
            "Expected stdout to contain {:?}.\nActual stdout:\n{}",
            expected,
            self.stdout
        );
    }

    /// Assert that stdout does NOT contain the given substring.
    pub fn assert_stdout_not_contains(&self, unexpected: &str) {
        assert!(
            !self.stdout.contains(unexpected),
            "Expected stdout to NOT contain {:?}.\nActual stdout:\n{}",
            unexpected,
            self.stdout
        );
    }

    /// Assert that stderr contains the given substring.
    pub fn assert_stderr_contains(&self, expected: &str) {
        assert!(
            self.stderr.contains(expected),
            "Expected stderr to contain {:?}.\nActual stderr:\n{}",
            expected,
            self.stderr
        );
    }

    /// Count occurrences of a pattern in stdout.
    pub fn count_in_stdout(&self, pattern: &str) -> usize {
        self.stdout.matches(pattern).count()
    }

    /// Get lines from stdout that match a predicate.
    pub fn stdout_lines_matching<F>(&self, predicate: F) -> Vec<&str>
    where
        F: Fn(&str) -> bool,
    {
        self.stdout.lines().filter(|l| predicate(l)).collect()
    }
}

/// Run taut as a subprocess on a project.
///
/// Returns the result with captured stdout/stderr.
pub fn run_taut(project: &TempProject, args: &[&str]) -> Result<TautResult> {
    run_taut_in_dir(project.path(), args)
}

/// Run taut in a specific directory.
pub fn run_taut_in_dir(dir: &Path, args: &[&str]) -> Result<TautResult> {
    // Find the taut binary - either in target/debug or target/release
    let taut_binary = find_taut_binary()?;

    let output = Command::new(&taut_binary)
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1") // Disable colors for easier testing
        .output()
        .with_context(|| format!("Failed to run taut: {}", taut_binary.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(TautResult {
        output,
        stdout,
        stderr,
        exit_code,
    })
}

/// Find the taut binary in target directory.
fn find_taut_binary() -> Result<PathBuf> {
    // Try debug build first, then release
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let manifest_path = PathBuf::from(manifest_dir);

    let debug_path = manifest_path.join("target/debug/taut");
    if debug_path.exists() {
        return Ok(debug_path);
    }

    let release_path = manifest_path.join("target/release/taut");
    if release_path.exists() {
        return Ok(release_path);
    }

    // Try current directory
    let debug_path = PathBuf::from("target/debug/taut");
    if debug_path.exists() {
        return Ok(debug_path);
    }

    let release_path = PathBuf::from("target/release/taut");
    if release_path.exists() {
        return Ok(release_path);
    }

    anyhow::bail!(
        "Could not find taut binary. Run `cargo build` first. \
        Searched: target/debug/taut, target/release/taut"
    )
}

/// Remove common leading whitespace from a string (dedent).
///
/// Similar to Python's `textwrap.dedent`.
pub fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();

    // Skip empty lines at the start
    let first_non_empty = lines.iter().position(|l| !l.trim().is_empty());
    let Some(start) = first_non_empty else {
        return String::new();
    };

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines[start..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    // Remove the common indentation from each line
    let result: Vec<String> = lines[start..]
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else if l.len() >= min_indent {
                l[min_indent..].to_string()
            } else {
                l.to_string()
            }
        })
        .collect();

    // Trim trailing empty lines
    let end = result
        .iter()
        .rposition(|l| !l.is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);

    result[..end].join("\n")
}

/// Helper to write a Python file with proper content.
pub fn write_python_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, dedent(content))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedent_basic() {
        let input = "
            def foo():
                pass
        ";
        let expected = "def foo():\n    pass";
        assert_eq!(dedent(input), expected);
    }

    #[test]
    fn test_dedent_preserves_relative_indent() {
        let input = "
            class Foo:
                def bar(self):
                    pass
        ";
        let expected = "class Foo:\n    def bar(self):\n        pass";
        assert_eq!(dedent(input), expected);
    }

    #[test]
    fn test_dedent_handles_empty_lines() {
        let input = "
            def foo():
                pass

            def bar():
                pass
        ";
        let expected = "def foo():\n    pass\n\ndef bar():\n    pass";
        assert_eq!(dedent(input), expected);
    }

    #[test]
    fn test_temp_project_write_file() {
        let mut project = TempProject::new().unwrap();
        let path = project
            .write_file("test_foo.py", "def test_ok(): pass")
            .unwrap();

        assert!(path.exists());
        assert!(project.file_exists("test_foo.py"));

        let content = project.read_file("test_foo.py").unwrap();
        assert_eq!(content, "def test_ok(): pass");
    }

    #[test]
    fn test_temp_project_subdirectory() {
        let mut project = TempProject::new().unwrap();
        project
            .write_file("subdir/test_nested.py", "def test_nested(): pass")
            .unwrap();

        assert!(project.file_exists("subdir/test_nested.py"));
    }
}
