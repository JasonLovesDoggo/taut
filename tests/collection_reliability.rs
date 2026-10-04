use std::{fs, path::PathBuf};
use taut::discovery::{
    collect_tests, extract_tests, extract_tests_from_file, find_test_files, selection_path,
};
use tempfile::TempDir;

#[test]
fn missing_root_is_an_error_even_beside_valid_tests() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_ok.py");
    fs::write(&file, "def test_ok(): pass\n").unwrap();
    let missing = dir.path().join("missing");
    let error = find_test_files(&[file, missing]).unwrap_err();
    assert!(error.to_string().contains("missing"));
}

#[test]
fn broken_source_cannot_produce_a_partial_success() {
    let dir = TempDir::new().unwrap();
    let good = dir.path().join("test_good.py");
    let broken = dir.path().join("test_broken.py");
    fs::write(&good, "def test_good(): pass\n").unwrap();
    fs::write(&broken, "def test_broken(\n").unwrap();
    let error = extract_tests(&[good, broken], Some("good")).unwrap_err();
    assert!(error.to_string().contains("test_broken.py"));
}

#[test]
fn read_failure_cannot_produce_a_partial_success() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_binary.py");
    fs::write(&file, [0xff, 0xfe]).unwrap();
    assert!(extract_tests(&[file], None).is_err());
}

#[test]
fn explicit_python_files_do_not_require_a_discovery_name() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("spec.py");
    fs::write(&file, "def test_spec(): pass\n").unwrap();
    assert!(
        find_test_files(&[dir.path().to_owned()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(find_test_files(&[file.clone()]).unwrap(), vec![file]);
}

#[test]
fn explicit_non_python_files_are_rejected() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_spec.txt");
    fs::write(&file, "def test_spec(): pass\n").unwrap();
    assert!(
        find_test_files(&[file])
            .unwrap_err()
            .to_string()
            .contains(".py")
    );
}

#[test]
fn conventional_suffix_files_are_discovered() {
    let dir = TempDir::new().unwrap();
    for name in [
        "math_test.py",
        "test_math.py",
        "_test_math.py",
        "math_test_helper.py",
    ] {
        fs::write(dir.path().join(name), "def test_spec(): pass\n").unwrap();
    }
    let names: Vec<_> = find_test_files(&[dir.path().to_owned()])
        .unwrap()
        .into_iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["_test_math.py", "math_test.py", "test_math.py"]);
}

#[test]
fn dependency_and_generated_directories_are_pruned() {
    let dir = TempDir::new().unwrap();
    for name in [
        ".git",
        ".venv",
        "venv",
        "__pycache__",
        "build",
        "dist",
        "target",
        "node_modules",
        ".tox",
    ] {
        fs::create_dir(dir.path().join(name)).unwrap();
        fs::write(dir.path().join(name).join("test_broken.py"), "invalid (").unwrap();
    }
    fs::write(dir.path().join("test_real.py"), "def test_real(): pass\n").unwrap();
    assert_eq!(
        collect_tests(&[dir.path().to_owned()], None).unwrap().len(),
        1
    );
    assert_eq!(
        find_test_files(&[dir.path().join(".venv")]).unwrap().len(),
        1
    );
}

#[test]
fn overlapping_roots_and_files_collect_once() {
    let dir = TempDir::new().unwrap();
    let child = dir.path().join("tests");
    fs::create_dir(&child).unwrap();
    let file = child.join("test_once.py");
    fs::write(&file, "def test_once(): pass\n").unwrap();
    for paths in [
        vec![dir.path().to_owned(), child.clone(), file.clone()],
        vec![file.clone(), child, dir.path().to_owned()],
    ] {
        let items = collect_tests(&paths, None).unwrap();
        assert_eq!(items.len(), 1);
    }
}

#[cfg(unix)]
#[test]
fn symlink_aliases_collect_once() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_once.py");
    let alias = dir.path().join("test_alias.py");
    fs::write(&file, "def test_once(): pass\n").unwrap();
    symlink(&file, &alias).unwrap();
    assert_eq!(
        collect_tests(&[alias, file, dir.path().to_owned()], None)
            .unwrap()
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn explicit_directory_aliases_collect_once_without_following_cycles() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tests");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("test_once.py"), "def test_once(): pass\n").unwrap();
    let alias = dir.path().join("alias");
    symlink(&root, &alias).unwrap();
    symlink(&root, root.join("cycle")).unwrap();
    assert_eq!(collect_tests(&[alias, root], None).unwrap().len(), 1);
}

#[test]
fn unicode_source_line_numbers_use_byte_offsets() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_unicode.py");
    fs::write(&file, "# 🐍 café\r\n\n@decorator\ndef test_雪():\n    pass\n\nclass TestUnicode:\n    async def test_async(self):\n        pass\n").unwrap();
    let items = extract_tests_from_file(&file).unwrap();
    assert_eq!(
        items.iter().map(|item| item.line).collect::<Vec<_>>(),
        [4, 8]
    );
}

#[test]
fn conditional_test_definitions_are_reported_instead_of_omitted() {
    for source in [
        "if True:\n    def test_hidden(): pass\n",
        "try:\n    pass\nexcept Exception:\n    def test_hidden(): pass\n",
        "class TestExample:\n    if True:\n        async def test_hidden(self): pass\n",
        "for item in []:\n    class TestHidden:\n        def test_hidden(self): pass\n",
    ] {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test_conditional.py");
        fs::write(&file, source).unwrap();
        let error = extract_tests_from_file(&file).unwrap_err().to_string();
        assert!(error.contains("unsupported"), "{error}");
        assert!(error.contains("test_conditional.py:"), "{error}");
    }
}

fn selector_fixture() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_select.py");
    fs::write(&file, "def test_a(): pass\ndef test_ab(): pass\nclass TestA:\n    def test_a(self): pass\n    async def test_b(self): pass\nclass TestAB:\n    def test_a(self): pass\n").unwrap();
    (dir, file)
}

#[test]
fn positional_selectors_match_exact_functions_classes_and_methods() {
    let (_dir, file) = selector_fixture();
    for (suffix, expected) in [
        ("test_a", vec!["test_a"]),
        ("TestA", vec!["test_a", "test_b"]),
        ("TestA::test_b", vec!["test_b"]),
    ] {
        let selector = PathBuf::from(format!("{}::{suffix}", file.display()));
        let items = collect_tests(&[selector.clone()], None).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.function.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(selection_path(&selector), file);
    }
}

#[test]
fn selectors_union_and_deduplicate_with_directory_selection() {
    let (dir, file) = selector_fixture();
    let paths = [
        PathBuf::from(format!("{}::TestA::test_b", file.display())),
        dir.path().to_owned(),
    ];
    assert_eq!(collect_tests(&paths, None).unwrap().len(), 5);
    let paths = [
        PathBuf::from(format!("{}::TestA", file.display())),
        PathBuf::from(format!("{}::TestA::test_b", file.display())),
    ];
    assert_eq!(collect_tests(&paths, None).unwrap().len(), 2);
}

#[test]
fn missing_or_malformed_selectors_fail() {
    let (_dir, file) = selector_fixture();
    for suffix in [
        "",
        "missing",
        "TestA::missing",
        "TestA::",
        "TestA::test_a::extra",
    ] {
        let selector = PathBuf::from(format!("{}::{suffix}", file.display()));
        assert!(collect_tests(&[selector], None).is_err(), "{suffix}");
    }
}

#[test]
fn selector_validity_is_checked_before_name_filter() {
    let (_dir, file) = selector_fixture();
    let selector = PathBuf::from(format!("{}::test_a", file.display()));
    assert!(
        collect_tests(&[selector], Some("test_ab"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn name_filter_preserves_directory_separators_in_file_part() {
    let filter = taut::filter::TestFilter::new("tests/unit/test_math.py::TestMath/*add").unwrap();
    assert!(filter.matches("tests/unit/test_math.py::TestMath::test_add"));
    assert!(!filter.matches("tests/integration/test_math.py::TestMath::test_add"));
}

#[test]
fn many_file_collection_preserves_order_and_propagates_errors() {
    let dir = TempDir::new().unwrap();
    for index in 0..160 {
        fs::write(
            dir.path().join(format!("test_{index:03}.py")),
            format!("def test_{index}(): pass\n"),
        )
        .unwrap();
    }
    let files = find_test_files(&[dir.path().to_owned()]).unwrap();
    let items = extract_tests(&files, None).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.function.clone())
            .collect::<Vec<_>>(),
        (0..160)
            .map(|index| format!("test_{index}"))
            .collect::<Vec<_>>()
    );
    fs::write(&files[32], "def test_broken(\n").unwrap();
    assert!(extract_tests(&files, Some("test_0")).is_err());
}
