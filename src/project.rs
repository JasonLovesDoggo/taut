//! Shared project boundaries for source snapshots and filesystem watches.

use std::path::{Path, PathBuf};

const CONFIG_FILES: [&str; 4] = ["pyproject.toml", "pytest.ini", "setup.cfg", "setup.py"];

pub(crate) fn is_configuration(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| CONFIG_FILES.iter().any(|candidate| name == *candidate))
}

pub(crate) fn root(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let directory = if path.is_dir() {
        path.as_path()
    } else {
        path.parent().unwrap_or(&path)
    };
    for ancestor in directory.ancestors() {
        // Never expand a selected subtree to the entire filesystem implicitly.
        if ancestor.parent().is_none() && ancestor != directory {
            break;
        }
        if CONFIG_FILES
            .iter()
            .any(|name| ancestor.join(name).is_file())
            || ancestor.join(".git").exists()
        {
            return ancestor.to_path_buf();
        }
    }
    // A plain tests tree conventionally imports sibling application modules.
    // Otherwise stay local instead of assuming an arbitrary cwd is a project.
    if let Some(parent) = directory
        .ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .is_some_and(|name| name == "tests" || name == "test")
        })
        .and_then(Path::parent)
        .filter(|parent| parent.parent().is_some())
    {
        return parent.to_path_buf();
    }
    directory.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_sentinels_and_git_worktrees_share_one_boundary() {
        for marker in CONFIG_FILES.into_iter().chain([".git"]) {
            let project = tempfile::tempdir().unwrap();
            std::fs::write(project.path().join(marker), "").unwrap();
            let selected = project.path().join("nested/specs");
            std::fs::create_dir_all(&selected).unwrap();
            let file = selected.join("test_example.py");
            std::fs::write(&file, "").unwrap();
            let expected = project.path().canonicalize().unwrap();
            assert_eq!(root(&selected), expected, "{marker}");
            assert_eq!(root(&file), expected, "{marker}");
        }
    }

    #[test]
    fn unconfigured_tests_subtrees_include_sibling_application_code() {
        let project = tempfile::tempdir().unwrap();
        let selected = project.path().join("tests/integration/api");
        std::fs::create_dir_all(&selected).unwrap();
        assert_eq!(root(&selected), project.path().canonicalize().unwrap());
    }

    #[test]
    fn unconfigured_arbitrary_directories_stay_local() {
        let project = tempfile::tempdir().unwrap();
        let selected = project.path().join("specs/integration");
        std::fs::create_dir_all(&selected).unwrap();
        assert_eq!(root(&selected), selected.canonicalize().unwrap());
    }

    #[test]
    fn nearest_project_boundary_wins() {
        let outer = tempfile::tempdir().unwrap();
        std::fs::write(outer.path().join("setup.cfg"), "").unwrap();
        let inner = outer.path().join("subpackage");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("pytest.ini"), "").unwrap();
        let selected = inner.join("tests");
        std::fs::create_dir(&selected).unwrap();
        assert_eq!(root(&selected), inner.canonicalize().unwrap());
    }
}
