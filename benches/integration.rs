//! Internal measurements. Use compare_execution.py for end-to-end CLI comparisons.
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use taut::discovery;
use taut::runner::{self, IsolationMode};

mod fixtures;
use fixtures::FixtureProject;

criterion_group!(
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_collection, bench_execution,
);
criterion_main!(benches);

fn collect(fixture: &FixtureProject) -> Vec<discovery::TestItem> {
    let paths = [fixture.dir.path().to_path_buf()];
    let files = discovery::find_test_files(&paths).expect("discover fixture files");
    assert_eq!(files.len(), fixture.test_files.len());
    let tests = discovery::extract_tests(&files, None).expect("collect fixture tests");
    assert_eq!(tests.len(), fixture.expected_tests);
    assert!(
        !tests.is_empty(),
        "an empty benchmark does not measure collection"
    );
    tests
}

fn bench_collection(c: &mut Criterion) {
    // Fixture generation is outside timing. Repetitions use warm filesystem caches.
    for (name, fixture) in [
        ("collection_100", FixtureProject::small()),
        ("collection_250", FixtureProject::medium()),
    ] {
        collect(&fixture);
        c.bench_function(name, |b| b.iter(|| black_box(collect(&fixture))));
    }
}

fn bench_execution(c: &mut Criterion) {
    for (name, fixture) in [
        ("noop_60", FixtureProject::noop()),
        ("cpu_100", FixtureProject::realistic()),
    ] {
        let tests = collect(&fixture);
        for (mode_name, mode) in [
            ("process_per_test", IsolationMode::ProcessPerTest),
            ("process_per_run", IsolationMode::ProcessPerRun),
        ] {
            c.bench_function(&format!("execute_{name}_{mode_name}"), |b| {
                b.iter(|| {
                    let results = runner::run_tests(&tests, true, Some(4), false, mode, |_| {})
                        .expect("benchmark runner must succeed");
                    assert_eq!(results.results.len(), fixture.expected_tests);
                    assert_eq!(results.passed_count(), fixture.expected_tests);
                    assert_eq!(results.failed_count(), 0);
                    assert_eq!(results.skipped_count(), 0);
                    black_box(results)
                });
            });
        }
    }
}
