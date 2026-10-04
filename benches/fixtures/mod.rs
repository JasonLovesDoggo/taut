use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct FixtureProject {
    pub dir: TempDir,
    pub test_files: Vec<PathBuf>,
    pub expected_tests: usize,
}

impl FixtureProject {
    pub fn small() -> Self {
        create_fixtures(20, 5, false)
    }

    pub fn medium() -> Self {
        create_fixtures(50, 5, false)
    }

    pub fn noop() -> Self {
        create_fixtures(30, 2, false)
    }

    pub fn realistic() -> Self {
        create_fixtures(20, 5, true)
    }
}

/// Every fixture contains exactly files * tests_per_file tests, even for odd counts.
fn create_fixtures(files: usize, tests_per_file: usize, cpu_work: bool) -> FixtureProject {
    let dir = TempDir::new().expect("create fixture directory");
    let mut test_files = Vec::new();
    for file in 0..files {
        let path = dir.path().join(format!("test_part_{file:03}.py"));
        let mut source = String::from("def parallel(function):\n    return function\n\n");
        for test in 0..tests_per_file {
            source.push_str(&format!("@parallel\ndef test_{test}():\n"));
            if cpu_work {
                source.push_str(
                    "    assert sum(value * value for value in range(1000)) == 332833500\n\n",
                );
            } else {
                source.push_str("    pass\n\n");
            }
        }
        fs::write(&path, source).expect("write fixture tests");
        test_files.push(path);
    }
    FixtureProject {
        dir,
        test_files,
        expected_tests: files * tests_per_file,
    }
}
