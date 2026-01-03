use criterion::{Criterion, criterion_group, criterion_main};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use taut::discovery;
use taut::runner::{self, IsolationMode};

mod fixtures;
use fixtures::FixtureProject;

criterion_group!(
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_cold_run_small,
             bench_cold_run_medium,
             bench_warm_run_small,
             bench_warm_run_medium,
             bench_incremental_small,
             bench_incremental_medium,
             bench_filtered_small,
             bench_filtered_medium,
             bench_noop_overhead,
             bench_execution_process_per_test,
             bench_execution_process_per_run,
             bench_execution_realistic_ppe,
             bench_execution_realistic_ppr,
);
criterion_main!(benches);

/// **Workflow 1: Cold Run (Small)**
/// First run on small project with no cache
fn bench_cold_run_small(c: &mut Criterion) {
    c.bench_function("cold_run_small", |b| {
        b.iter_batched(
            || FixtureProject::small(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 1: Cold Run (Medium)**
/// First run on medium project with no cache
fn bench_cold_run_medium(c: &mut Criterion) {
    c.bench_function("cold_run_medium", |b| {
        b.iter_batched(
            || FixtureProject::medium(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 2: Warm Run (Small)**
/// Run when nothing changed - all tests in cache
fn bench_warm_run_small(c: &mut Criterion) {
    c.bench_function("warm_run_small", |b| {
        b.iter_batched(
            || FixtureProject::small(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // First run to populate data
                let _ = discovery::extract_tests(&project_dir, None);
                // Second run is "warm"
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 2: Warm Run (Medium)**
/// Run when nothing changed - all tests in cache
fn bench_warm_run_medium(c: &mut Criterion) {
    c.bench_function("warm_run_medium", |b| {
        b.iter_batched(
            || FixtureProject::medium(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // First run to populate data
                let _ = discovery::extract_tests(&project_dir, None);
                // Second run is "warm"
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 3: Incremental (Small)**
/// Modify one file, rerun tests
fn bench_incremental_small(c: &mut Criterion) {
    c.bench_function("incremental_small", |b| {
        b.iter_batched(
            || {
                let fixture = FixtureProject::small();
                // Modify a test file to simulate change
                if let Some(test_file) = fixture.test_files.first() {
                    let content = fs::read_to_string(test_file).unwrap_or_default();
                    let modified = format!("# Modified\n{}", content);
                    let _ = fs::write(test_file, modified);
                }
                fixture
            },
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // Rerun after modification
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 3: Incremental (Medium)**
/// Modify one file, rerun tests
fn bench_incremental_medium(c: &mut Criterion) {
    c.bench_function("incremental_medium", |b| {
        b.iter_batched(
            || {
                let fixture = FixtureProject::medium();
                // Modify a test file to simulate change
                if let Some(test_file) = fixture.test_files.first() {
                    let content = fs::read_to_string(test_file).unwrap_or_default();
                    let modified = format!("# Modified\n{}", content);
                    let _ = fs::write(test_file, modified);
                }
                fixture
            },
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // Rerun after modification
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 4: Filtered Execution (Small)**
/// Run with `-k` filter matching subset of tests
fn bench_filtered_small(c: &mut Criterion) {
    c.bench_function("filtered_small", |b| {
        b.iter_batched(
            || FixtureProject::small(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // Filter to match ~10% of tests (pattern that matches some but not all)
                let _ = discovery::extract_tests(&project_dir, Some("test_api"));
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 4: Filtered Execution (Medium)**
/// Run with `-k` filter matching subset of tests
fn bench_filtered_medium(c: &mut Criterion) {
    c.bench_function("filtered_medium", |b| {
        b.iter_batched(
            || FixtureProject::medium(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // Filter to match ~10% of tests
                let _ = discovery::extract_tests(&project_dir, Some("test_api"));
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 5: Execution Overhead**
/// Measure taut's overhead with minimal-work tests
/// - noop: just `pass`, isolates discovery/selection overhead
/// - sleep: `time.sleep(0.001)`, allows calculating IPC overhead
fn bench_noop_overhead(c: &mut Criterion) {
    c.bench_function("overhead_noop", |b| {
        b.iter_batched(
            || FixtureProject::noop(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                // Discover noop tests (minimal execution needed)
                let _ = discovery::extract_tests(&project_dir, None);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 6: Execution with Process-Per-Test**
/// Measure overhead of spawning Python for each test
/// Each test gets its own Python process (~50ms startup overhead expected)
fn bench_execution_process_per_test(c: &mut Criterion) {
    c.bench_function("execution_process_per_test", |b| {
        b.iter_batched(
            || FixtureProject::noop(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let tests = discovery::extract_tests(&project_dir, None).unwrap_or_default();

                let counter = Arc::new(AtomicUsize::new(0));
                let _ = runner::run_tests(
                    &tests,
                    true,  // parallel
                    None,  // default jobs
                    false, // no coverage
                    IsolationMode::ProcessPerTest,
                    |_result| {
                        counter.fetch_add(1, Ordering::Relaxed);
                    },
                );
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 7: Execution with Worker Pool (Process-Per-Run)**
/// Measure efficiency of worker pool with warm reuse
/// Tests share Python processes (~1-5ms overhead per test expected)
fn bench_execution_process_per_run(c: &mut Criterion) {
    c.bench_function("execution_process_per_run", |b| {
        b.iter_batched(
            || FixtureProject::noop(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let tests = discovery::extract_tests(&project_dir, None).unwrap_or_default();

                let counter = Arc::new(AtomicUsize::new(0));
                let _ = runner::run_tests(
                    &tests,
                    true,  // parallel
                    None,  // default jobs
                    false, // no coverage
                    IsolationMode::ProcessPerRun,
                    |_result| {
                        counter.fetch_add(1, Ordering::Relaxed);
                    },
                );
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 8: Realistic Tests with Process-Per-Test**
/// Tests with actual work (math, string ops, JSON parsing)
fn bench_execution_realistic_ppe(c: &mut Criterion) {
    c.bench_function("execution_realistic_ppe", |b| {
        b.iter_batched(
            || FixtureProject::realistic(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let tests = discovery::extract_tests(&project_dir, None).unwrap_or_default();

                let counter = Arc::new(AtomicUsize::new(0));
                let _ = runner::run_tests(
                    &tests,
                    true,  // parallel
                    None,  // default jobs
                    false, // no coverage
                    IsolationMode::ProcessPerTest,
                    |_result| {
                        counter.fetch_add(1, Ordering::Relaxed);
                    },
                );
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// **Workflow 9: Realistic Tests with Process-Per-Run (Worker Pool)**
/// Tests with actual work using worker pool reuse
fn bench_execution_realistic_ppr(c: &mut Criterion) {
    c.bench_function("execution_realistic_ppr", |b| {
        b.iter_batched(
            || FixtureProject::realistic(),
            |fixture| {
                let project_dir = vec![fixture.dir.path().to_path_buf()];
                let tests = discovery::extract_tests(&project_dir, None).unwrap_or_default();

                let counter = Arc::new(AtomicUsize::new(0));
                let _ = runner::run_tests(
                    &tests,
                    true,  // parallel
                    None,  // default jobs
                    false, // no coverage
                    IsolationMode::ProcessPerRun,
                    |_result| {
                        counter.fetch_add(1, Ordering::Relaxed);
                    },
                );
            },
            criterion::BatchSize::SmallInput,
        );
    });
}
