use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct FixtureProject {
    pub dir: TempDir,
    pub test_files: Vec<PathBuf>,
}

impl FixtureProject {
    /// Small fixture: ~20 files, ~100 tests (quick feedback)
    pub fn small() -> Self {
        create_fixtures(20, 5, false)
    }

    /// Medium fixture: ~50 files, ~250 tests (realistic size)
    pub fn medium() -> Self {
        create_fixtures(50, 5, false)
    }

    /// Noop fixture: ~30 files with minimal-work tests (for overhead measurement)
    pub fn noop() -> Self {
        create_fixtures(30, 2, true)
    }
}

/// Generate fixture files programmatically
fn create_fixtures(num_files: usize, tests_per_file: usize, noop_mode: bool) -> FixtureProject {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = dir.path();

    // Create subdirectories to mimic realistic project structure
    let modules = ["api", "models", "views", "services", "utils"];
    for module in &modules {
        let module_path = base_path.join(module);
        fs::create_dir(&module_path).expect(&format!("Failed to create {} dir", module));

        // Distribute files across modules
        let files_per_module = num_files / modules.len();
        for i in 0..files_per_module {
            let filename = format!("test_{}_{}.py", module, i);
            let file_path = module_path.join(&filename);
            let content = generate_test_file(&filename, module, tests_per_file, noop_mode);
            fs::write(&file_path, content).expect(&format!("Failed to write {}", filename));
        }
    }

    // Collect all test files
    let mut test_files = Vec::new();
    for module in &modules {
        let module_path = base_path.join(module);
        if let Ok(entries) = fs::read_dir(&module_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "py") {
                    test_files.push(path);
                }
            }
        }
    }
    test_files.sort();

    FixtureProject { dir, test_files }
}

/// Generate a single test file with realistic Python code
fn generate_test_file(filename: &str, module: &str, test_count: usize, noop_mode: bool) -> String {
    let mut content = String::new();
    content.push_str(&format!("# {}\n", filename));
    content.push_str("\"\"\"Test module.\"\"\"\n\n");

    if noop_mode {
        // For overhead measurement: tests that do minimal work
        content.push_str("import time\n\n");

        for i in 0..test_count {
            content.push_str(&format!(
                "def test_noop_{}():\n    \"\"\"Minimal test.\"\"\"\n    pass\n\n",
                i
            ));

            content.push_str(&format!(
                "def test_sleep_{}():\n    \"\"\"Sleep test for overhead measurement.\"\"\"\n    time.sleep(0.001)\n\n",
                i
            ));
        }
    } else {
        // For realistic workloads: tests with various patterns
        // Plain function tests
        for i in 0..test_count / 2 {
            content.push_str(&format!(
                "def test_{}_{}():\n    \"\"\"Test {}.\"\"\"\n    assert True\n\n",
                module, i, i
            ));
        }

        // Class-based tests
        let class_name = format!(
            "Test{}",
            module
                .chars()
                .next()
                .unwrap()
                .to_uppercase()
                .chain(module.chars().skip(1))
                .collect::<String>()
        );

        content.push_str(&format!("class {}:\n", class_name));
        content.push_str("    \"\"\"Test class.\"\"\"\n\n");

        for i in 0..test_count / 2 {
            content.push_str(&format!(
                "    def test_method_{}(self):\n        \"\"\"Test method {}.\"\"\"\n        assert True\n\n",
                i, i
            ));
        }
    }

    content
}
