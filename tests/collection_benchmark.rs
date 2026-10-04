//! Run with `cargo test --release --test collection_benchmark -- --ignored --nocapture`.
//! Fixture construction is outside timing; count assertions prevent failed reads
//! or empty collections from masquerading as a performance improvement.
use std::{fs, hint::black_box, time::Instant};
use taut::discovery::{extract_tests, find_test_files};
use tempfile::TempDir;

#[test]
#[ignore = "manual collection performance benchmark"]
fn large_collection() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_large.py");
    let source: String = (0..10_000)
        .map(|index| format!("def test_{index}():\n    assert {index} == {index}\n\n"))
        .collect();
    fs::write(&file, source).unwrap();
    let files = vec![file];
    assert_eq!(extract_tests(&files, None).unwrap().len(), 10_000);
    let started = Instant::now();
    for _ in 0..5 {
        assert_eq!(
            black_box(extract_tests(&files, None).unwrap()).len(),
            10_000
        );
    }
    eprintln!(
        "10,000 tests / one file: {:.3} ms/collection",
        started.elapsed().as_secs_f64() * 1000.0 / 5.0
    );

    let dir = TempDir::new().unwrap();
    for file in 0..500 {
        let source: String = (0..20)
            .map(|index| format!("async def test_{index}():\n    assert {index} == {index}\n\n"))
            .collect();
        fs::write(dir.path().join(format!("test_{file}.py")), source).unwrap();
    }
    let paths = vec![dir.path().to_owned()];
    let started = Instant::now();
    for _ in 0..5 {
        let files = find_test_files(&paths).unwrap();
        assert_eq!(files.len(), 500);
        assert_eq!(
            black_box(extract_tests(&files, None).unwrap()).len(),
            10_000
        );
    }
    eprintln!(
        "10,000 async tests / 500 files with walking: {:.3} ms/collection",
        started.elapsed().as_secs_f64() * 1000.0 / 5.0
    );
}

#[test]
#[ignore = "manual parallel collection crossover benchmark"]
fn parallel_collection_crossover() {
    use rayon::prelude::*;
    use taut::discovery::extract_tests_from_file;
    for tests_per_file in [1, 20] {
        for count in [8, 16, 32, 64, 128, 500] {
            let dir = TempDir::new().unwrap();
            let source: String = (0..tests_per_file)
                .map(|index| format!("def test_{index}():\n    assert {index} == {index}\n"))
                .collect();
            for file in 0..count {
                fs::write(dir.path().join(format!("test_{file}.py")), &source).unwrap();
            }
            let files = find_test_files(&[dir.path().to_owned()]).unwrap();
            let sequential = || {
                files
                    .iter()
                    .map(|file| extract_tests_from_file(file).unwrap().len())
                    .sum::<usize>()
            };
            let parallel = || {
                files
                    .par_iter()
                    .map(|file| extract_tests_from_file(file).unwrap().len())
                    .sum::<usize>()
            };
            let cold = Instant::now();
            assert_eq!(parallel(), count * tests_per_file);
            let cold_parallel = cold.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            for _ in 0..20 {
                assert_eq!(black_box(sequential()), count * tests_per_file);
            }
            let serial_ms = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
            let started = Instant::now();
            for _ in 0..20 {
                assert_eq!(black_box(parallel()), count * tests_per_file);
            }
            let parallel_ms = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
            eprintln!(
                "{count} files x {tests_per_file}: serial {serial_ms:.3} ms, parallel {parallel_ms:.3} ms, first parallel {cold_parallel:.3} ms"
            );
        }
    }
}

#[test]
#[ignore = "manual inherited collection benchmark"]
fn inherited_collection() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test_inherited.py");
    let mut source = "class Base:\n".to_owned();
    for method in 0..40 {
        source.push_str(&format!("    def test_{method}(self): assert True\n"));
    }
    for class in 0..250 {
        source.push_str(&format!("class Test{class}(Base): pass\n"));
    }
    fs::write(&file, source).unwrap();
    let started = Instant::now();
    for _ in 0..20 {
        assert_eq!(
            black_box(extract_tests(std::slice::from_ref(&file), None).unwrap()).len(),
            10_000
        );
    }
    eprintln!(
        "10,000 inherited tests / 250 classes: {:.3} ms/collection",
        started.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
}
