//! Strict, project-local configuration from `[tool.taut]` in pyproject.toml.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    #[serde(alias = "max_workers")]
    pub max_workers: Option<usize>,
    pub isolation: Option<String>,
    pub python: Option<PathBuf>,
    #[serde(alias = "async_concurrency")]
    pub async_concurrency: Option<usize>,
    pub timeout: Option<f64>,
    #[serde(alias = "fail_fast")]
    pub fail_fast: bool,
}

pub(crate) struct ProjectConfig {
    pub root: PathBuf,
    pub path: Option<PathBuf>,
    pub options: Config,
}

impl Config {
    /// Load configuration from the nearest project boundary.
    pub fn load(start: &Path) -> Result<Self> {
        Ok(Self::load_project(start, false)?.options)
    }

    pub(crate) fn load_project(start: &Path, ignore_settings: bool) -> Result<ProjectConfig> {
        let start = start
            .canonicalize()
            .with_context(|| format!("cannot access {}", start.display()))?;
        let root = crate::project::root(&start);
        let path = root.join("pyproject.toml");
        let path = path.exists().then_some(path);
        // Project boundaries and environment discovery still use the file as a
        // marker when --no-config skips settings, including malformed TOML.
        let options = if let Some(path) = path.as_ref().filter(|_| !ignore_settings) {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let mut config = Self::parse(&content)
                .with_context(|| format!("invalid configuration in {}", path.display()))?;
            if let Some(python) = &config.python
                && python.is_relative()
                && python.components().count() > 1
            {
                config.python = Some(root.join(python));
            }
            config
        } else {
            Self::default()
        };
        Ok(ProjectConfig {
            root,
            path,
            options,
        })
    }

    fn parse(content: &str) -> Result<Self> {
        let document: toml::Value = content.parse().context("invalid TOML")?;
        let Some(section) = document.get("tool").and_then(|tool| tool.get("taut")) else {
            return Ok(Self::default());
        };
        let config: Self = section
            .clone()
            .try_into()
            .context("invalid [tool.taut] options")?;
        if config.max_workers == Some(0) {
            bail!("max-workers must be greater than zero");
        }
        if config.async_concurrency == Some(0) {
            bail!("async-concurrency must be greater than zero");
        }
        if let Some(value) = &config.isolation
            && !matches!(value.as_str(), "process-per-run" | "process-per-test")
        {
            bail!("isolation must be process-per-run or process-per-test");
        }
        if let Some(timeout) = config.timeout {
            parse_timeout(timeout)?;
        }
        if config
            .python
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
        {
            bail!("python must name an interpreter or executable path");
        }
        Ok(config)
    }
}

pub fn parse_timeout(seconds: f64) -> Result<Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("timeout must be a finite number greater than zero (seconds)");
    }
    let duration = Duration::try_from_secs_f64(seconds).context("timeout is too large")?;
    if duration.is_zero() {
        bail!("timeout must be at least one nanosecond");
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_config_spellings() {
        for key in ["max-workers", "max_workers"] {
            let config = Config::parse(&format!("[tool.taut]\n{key} = 4")).unwrap();
            assert_eq!(config.max_workers, Some(4));
        }
    }

    #[test]
    fn absent_section_uses_defaults() {
        assert!(
            Config::parse("[project]\nname = 'example'")
                .unwrap()
                .max_workers
                .is_none()
        );
    }

    #[test]
    fn rejects_invalid_settings_instead_of_ignoring_them() {
        for setting in [
            "max_workers = 0",
            "max_workers = -1",
            "max_workers = '4'",
            "async-concurrency = 0",
            "timeout = 0",
            "timeout = nan",
            "timeout = inf",
            "isolation = 'proces'",
            "max_worker = 4",
            "python = ''",
        ] {
            assert!(
                Config::parse(&format!("[tool.taut]\n{setting}")).is_err(),
                "{setting}"
            );
        }
    }

    #[test]
    fn stops_at_nearest_project_boundary() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("pyproject.toml"),
            "[tool.taut]\nmax-workers = 9",
        )
        .unwrap();
        let child = temp.path().join("child");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("pyproject.toml"), "[project]\nname = 'child'").unwrap();
        assert_eq!(Config::load(&child).unwrap().max_workers, None);
    }

    #[test]
    fn malformed_project_is_actionable() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("pyproject.toml"), "[tool.taut\n").unwrap();
        assert!(format!("{:#}", Config::load(temp.path()).unwrap_err()).contains("pyproject.toml"));
    }
}
