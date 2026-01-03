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

    /// Realistic fixture: ~20 files with actual test work (computations, assertions)
    pub fn realistic() -> Self {
        create_realistic_fixtures(20, 5)
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

/// Create realistic test fixtures with actual test work
fn create_realistic_fixtures(num_files: usize, tests_per_file: usize) -> FixtureProject {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = dir.path();

    // Create subdirectories
    let modules = ["math", "string", "list", "dict", "file"];
    for module in &modules {
        let module_path = base_path.join(module);
        fs::create_dir(&module_path).expect(&format!("Failed to create {} dir", module));

        // Distribute files across modules
        let files_per_module = num_files / modules.len();
        for i in 0..files_per_module {
            let filename = format!("test_{}_{}.py", module, i);
            let file_path = module_path.join(&filename);
            let content = generate_realistic_test_file(&filename, module, tests_per_file);
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

/// Generate a test file with actual work (math, string operations, etc)
fn generate_realistic_test_file(filename: &str, module: &str, test_count: usize) -> String {
    let mut content = String::new();
    content.push_str(&format!("# {}\n", filename));
    content.push_str("\"\"\"Tests with actual work.\"\"\"\n\n");

    match module {
        "math" => {
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "def test_math_{}():\n    result = sum(range(1000))\n    assert result == 499500\n\n",
                    i
                ));
            }
            content.push_str("class TestMath:\n");
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "    def test_sqrt_{}(self):\n        import math\n        assert abs(math.sqrt({}) - {:.1}) < 0.01\n\n",
                    i, (i * i) as f64, i as f64
                ));
            }
        }
        "string" => {
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "def test_string_{}():\n    s = 'test' * {}\n    assert len(s) == {}\n\n",
                    i,
                    (i + 1),
                    4 * (i + 1)
                ));
            }
            content.push_str("class TestString:\n");
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "    def test_format_{}(self):\n        result = '{{}}' * {}\n        assert len(result) == {}\n\n",
                    i,
                    (i + 1),
                    i + 1
                ));
            }
        }
        "list" => {
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "def test_list_{}():\n    lst = list(range({}))\n    assert len(lst) == {}\n\n",
                    i,
                    (i * 100 + 100),
                    (i * 100 + 100)
                ));
            }
            content.push_str("class TestList:\n");
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "    def test_sort_{}(self):\n        lst = list(range({}, 0, -1))\n        lst.sort()\n        assert lst[0] == 1\n\n",
                    i, (i * 50 + 50)
                ));
            }
        }
        "dict" => {
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "def test_dict_{}():\n    d = {{i: i*2 for i in range({})}}\n    assert len(d) == {}\n\n",
                    i, (i + 1), (i + 1)
                ));
            }
            content.push_str("class TestDict:\n");
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "    def test_keys_{}(self):\n        d = {{'a': 1, 'b': 2}}\n        assert len(d.keys()) == 2\n\n",
                    i
                ));
            }
        }
        "file" => {
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "def test_json_{}():\n    import json\n    data = json.dumps({{'key': 'value{}'}})\n    assert 'key' in data\n\n",
                    i, i
                ));
            }
            content.push_str("class TestFile:\n");
            for i in 0..test_count / 2 {
                content.push_str(&format!(
                    "    def test_parse_{}(self):\n        import json\n        s = '{{\"a\": {}}}'\n        data = json.loads(s)\n        assert data['a'] == {}\n\n",
                    i, i, i
                ));
            }
        }
        _ => {
            // Fallback
            for i in 0..test_count {
                content.push_str(&format!(
                    "def test_{}():\n    assert {} + 1 == {}\n\n",
                    i,
                    i,
                    i + 1
                ));
            }
        }
    }

    content
}
