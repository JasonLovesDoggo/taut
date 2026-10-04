//! Interpreter selection shared by the CLI and the public runner API.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    Cli,
    Config,
    TautPython,
    VirtualEnv,
    ProjectVenv,
    Launcher,
    Path,
}

impl Source {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cli => "--python",
            Self::Config => "[tool.taut].python",
            Self::TautPython => "TAUT_PYTHON",
            Self::VirtualEnv => "VIRTUAL_ENV",
            Self::ProjectVenv => "project .venv",
            Self::Launcher => "launcher sibling",
            Self::Path => "PATH",
        }
    }
}

pub(crate) struct Python {
    pub path: PathBuf,
    pub source: Source,
}

impl Python {
    /// Resolve executables without canonicalizing a virtualenv's Python symlink.
    pub(crate) fn resolve(mut self) -> Result<Self> {
        let path = if self.path.as_os_str().is_empty() {
            None
        } else if self.path.components().count() != 1 || self.path.is_absolute() {
            is_executable(&self.path).then(|| self.path.clone())
        } else {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths).find_map(|directory| {
                    let candidate = directory.join(&self.path);
                    if is_executable(&candidate) {
                        return Some(candidate);
                    }
                    #[cfg(windows)]
                    if candidate.extension().is_none() {
                        let executable = candidate.with_extension("exe");
                        if is_executable(&executable) {
                            return Some(executable);
                        }
                    }
                    None
                })
            })
        };
        let Some(path) = path else {
            bail!(
                "Python from {} is not an executable: {}",
                self.source.label(),
                self.path.display()
            );
        };
        self.path = std::path::absolute(&path)
            .with_context(|| format!("cannot resolve Python at {}", path.display()))?;
        Ok(self)
    }
}

pub(crate) fn select(explicit: Option<(PathBuf, Source)>, project_root: &Path) -> Python {
    let (path, source) = if let Some(explicit) = explicit {
        explicit
    } else if let Some(path) = std::env::var_os("TAUT_PYTHON") {
        (path.into(), Source::TautPython)
    } else if let Some(venv) = std::env::var_os("VIRTUAL_ENV") {
        (PathBuf::from(venv).join(venv_python()), Source::VirtualEnv)
    } else if project_root.join(".venv").exists() {
        (
            project_root.join(".venv").join(venv_python()),
            Source::ProjectVenv,
        )
    } else if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|directory| directory.join(python_name()))
        })
        .filter(|path| path.is_file())
    {
        (path, Source::Launcher)
    } else {
        (
            PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
            Source::Path,
        )
    };
    Python { path, source }
}

fn python_name() -> &'static str {
    if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    }
}

fn venv_python() -> &'static str {
    if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
