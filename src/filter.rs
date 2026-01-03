//! Test filtering with glob patterns and file::test syntax.
//!
//! Supports Go-style filtering patterns:
//! - `test_user` - simple substring match
//! - `test_*login` - glob pattern with wildcard
//! - `test_user/*` - class/subtest syntax
//! - `test_login.py::test_user` - file-specific filtering

use regex::Regex;

/// A compiled test filter that can match test IDs.
#[derive(Debug)]
pub struct TestFilter {
    /// Original pattern string
    pattern: String,
    /// Compiled regex for matching
    regex: Regex,
    /// Optional file pattern (for file.py::test syntax)
    file_pattern: Option<Regex>,
}

impl TestFilter {
    /// Create a new filter from a glob pattern.
    ///
    /// Patterns:
    /// - `test_foo` → matches any test containing "test_foo"
    /// - `test_*foo` → glob wildcard, matches test_bar_foo, test_foo, etc.
    /// - `TestClass/*` → matches all methods in TestClass
    /// - `file.py::test_foo` → matches test_foo only in file.py
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        // Handle file.py::test syntax
        if let Some((file_part, test_part)) = pattern.split_once("::") {
            let file_regex = glob_to_regex(file_part)?;
            let test_regex = glob_to_regex(test_part)?;
            Ok(Self {
                pattern: pattern.to_string(),
                regex: test_regex,
                file_pattern: Some(file_regex),
            })
        } else {
            let regex = glob_to_regex(pattern)?;
            Ok(Self {
                pattern: pattern.to_string(),
                regex,
                file_pattern: None,
            })
        }
    }

    /// Check if a test ID matches this filter.
    ///
    /// Test ID format: `path/to/file.py::TestClass::test_method` or `path/to/file.py::test_func`
    pub fn matches(&self, test_id: &str) -> bool {
        // Split test ID into file and test parts
        let (file_part, test_part) = if let Some(idx) = test_id.find("::") {
            (&test_id[..idx], &test_id[idx + 2..])
        } else {
            ("", test_id)
        };

        // If we have a file filter, check it first
        if let Some(ref file_regex) = self.file_pattern {
            if !file_regex.is_match(file_part) {
                return false;
            }
        }

        // Match the test part
        self.regex.is_match(test_part)
    }

    /// Get the original pattern string.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

/// Convert a glob pattern to a case-insensitive regex.
///
/// Glob patterns:
/// - `*` → matches any sequence of characters (except ::)
/// - `?` → matches any single character
/// - Other characters are escaped
fn glob_to_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let mut regex_str = String::with_capacity(pattern.len() * 2 + 4);

    // Case-insensitive matching (like Go's -run and pytest's -k)
    regex_str.push_str("(?i)");

    // Don't anchor at start - allow substring matching by default
    // This matches Go's -run behavior

    for c in pattern.chars() {
        match c {
            '*' => regex_str.push_str("[^:]*"), // Match anything except ::
            '?' => regex_str.push('.'),
            '.' => regex_str.push_str("\\."),
            '^' => regex_str.push_str("\\^"),
            '$' => regex_str.push_str("\\$"),
            '|' => regex_str.push_str("\\|"),
            '(' => regex_str.push_str("\\("),
            ')' => regex_str.push_str("\\)"),
            '[' => regex_str.push_str("\\["),
            ']' => regex_str.push_str("\\]"),
            '{' => regex_str.push_str("\\{"),
            '}' => regex_str.push_str("\\}"),
            '+' => regex_str.push_str("\\+"),
            '\\' => regex_str.push_str("\\\\"),
            '/' => regex_str.push_str("::"), // Treat / as :: for subtest syntax
            _ => regex_str.push(c),
        }
    }

    Regex::new(&regex_str)
}

/// Filter a list of test IDs by a pattern.
pub fn filter_tests<'a>(
    test_ids: impl Iterator<Item = &'a str>,
    pattern: &str,
) -> Result<Vec<&'a str>, regex::Error> {
    let filter = TestFilter::new(pattern)?;
    Ok(test_ids.filter(|id| filter.matches(id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_substring_match() {
        let filter = TestFilter::new("test_user").unwrap();
        assert!(filter.matches("test_user"));
        assert!(filter.matches("test_user_login"));
        assert!(filter.matches("test_user_logout"));
        assert!(filter.matches("tests/auth.py::test_user_login"));
        assert!(!filter.matches("test_admin"));
    }

    #[test]
    fn test_glob_wildcard() {
        let filter = TestFilter::new("test_*login").unwrap();
        assert!(filter.matches("test_login"));
        assert!(filter.matches("test_user_login"));
        assert!(filter.matches("test_admin_login"));
        assert!(!filter.matches("test_logout"));
    }

    #[test]
    fn test_subtest_syntax() {
        // Using / to mean :: (class/method)
        let filter = TestFilter::new("TestUser/*").unwrap();
        assert!(filter.matches("tests/auth.py::TestUser::test_login"));
        assert!(filter.matches("tests/auth.py::TestUser::test_logout"));
        assert!(!filter.matches("tests/auth.py::TestAdmin::test_login"));
    }

    #[test]
    fn test_file_specific() {
        let filter = TestFilter::new("auth.py::test_login").unwrap();
        assert!(filter.matches("tests/auth.py::test_login"));
        assert!(filter.matches("tests/auth.py::test_login_user"));
        assert!(!filter.matches("tests/user.py::test_login"));
    }

    #[test]
    fn test_file_with_glob() {
        let filter = TestFilter::new("auth*::test_*").unwrap();
        assert!(filter.matches("tests/auth.py::test_login"));
        assert!(filter.matches("tests/auth_test.py::test_logout"));
        assert!(!filter.matches("tests/user.py::test_login"));
    }

    #[test]
    fn test_question_mark_wildcard() {
        let filter = TestFilter::new("test_?").unwrap();
        assert!(filter.matches("test_a"));
        assert!(filter.matches("test_1"));
        // Note: test_ab still matches because "test_?" appears as substring
        // This is Go-style substring matching behavior
        assert!(filter.matches("test_ab"));
        // But "test_" alone doesn't match (requires exactly one char after)
        assert!(!filter.matches("test_"));
    }

    #[test]
    fn test_special_chars_escaped() {
        let filter = TestFilter::new("test.foo").unwrap();
        assert!(filter.matches("test.foo"));
        assert!(!filter.matches("testXfoo")); // . is literal, not regex wildcard
    }

    #[test]
    fn test_empty_pattern_matches_all() {
        let filter = TestFilter::new("").unwrap();
        assert!(filter.matches("test_anything"));
        assert!(filter.matches("tests/foo.py::test_bar"));
    }
}
