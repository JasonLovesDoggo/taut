//! Configuration loading from pyproject.toml.
//!
//! Reads taut configuration from [tool.taut] section in pyproject.toml.

use std::path::Path;

/// Taut configuration options.
#[derive(Debug, Default)]
pub struct Config {
    /// Maximum number of worker processes.
    pub max_workers: Option<usize>,
}

impl Config {
    /// Load configuration from pyproject.toml in the given directory.
    /// Falls back to parent directories until a pyproject.toml is found.
    /// Returns default config if no pyproject.toml exists.
    pub fn load(start_dir: &Path) -> Self {
        let mut dir = if start_dir.is_file() {
            start_dir.parent().map(Path::to_path_buf)
        } else {
            Some(start_dir.to_path_buf())
        };

        while let Some(d) = dir {
            let pyproject = d.join("pyproject.toml");
            if pyproject.exists() {
                if let Ok(content) = std::fs::read_to_string(&pyproject) {
                    if let Some(config) = Self::parse(&content) {
                        return config;
                    }
                }
            }
            dir = d.parent().map(Path::to_path_buf);
        }

        Self::default()
    }

    /// Parse configuration from pyproject.toml content.
    fn parse(content: &str) -> Option<Self> {
        let doc: toml::Value = content.parse().ok()?;
        let tool = doc.get("tool")?;
        let taut = tool.get("taut")?;

        let max_workers = taut
            .get("max_workers")
            .and_then(|v| v.as_integer())
            .map(|n| n as usize);

        Some(Self { max_workers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_max_workers() {
        let content = r#"
[tool.taut]
max_workers = 4
"#;
        let config = Config::parse(content).unwrap();
        assert_eq!(config.max_workers, Some(4));
    }

    #[test]
    fn parse_empty_taut_section() {
        let content = r#"
[tool.taut]
"#;
        let config = Config::parse(content).unwrap();
        assert_eq!(config.max_workers, None);
    }

    #[test]
    fn parse_no_taut_section() {
        let content = r#"
[tool.other]
foo = "bar"
"#;
        let config = Config::parse(content);
        assert!(config.is_none());
    }

    #[test]
    fn parse_no_tool_section() {
        let content = r#"
[project]
name = "myproject"
"#;
        let config = Config::parse(content);
        assert!(config.is_none());
    }
}
