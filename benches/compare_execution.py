#!/usr/bin/env python3
"""Compare taut execution modes vs pytest on actual test runs."""

import subprocess
import tempfile
import os
import time
import json
from pathlib import Path

def create_noop_test_project(num_tests: int = 60) -> Path:
    """Create test project with noop tests."""
    tmpdir = Path(tempfile.mkdtemp())

    # Create test file with noop tests
    test_file = tmpdir / "test_noop.py"
    content = 'import time\n\n'

    # Half plain functions, half class methods
    for i in range(num_tests // 2):
        content += f'def test_noop_{i}():\n    pass\n\n'

    content += 'class TestNoop:\n'
    for i in range(num_tests // 2):
        content += f'    def test_method_{i}(self):\n        pass\n\n'

    test_file.write_text(content)
    return tmpdir


def benchmark_taut_process_per_test(project_dir: Path, iterations: int = 3) -> float:
    """Benchmark taut with process-per-test isolation."""
    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        result = subprocess.run(
            ["taut", "tests", "--isolation", "process-per-test", str(project_dir)],
            capture_output=True,
            timeout=60,
            cwd=str(project_dir)
        )
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    return avg * 1000  # ms


def benchmark_taut_process_per_run(project_dir: Path, iterations: int = 3) -> float:
    """Benchmark taut with process-per-run isolation (worker pool)."""
    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        result = subprocess.run(
            ["taut", "tests", "--isolation", "process-per-run", str(project_dir)],
            capture_output=True,
            timeout=60,
            cwd=str(project_dir)
        )
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    return avg * 1000  # ms


def benchmark_pytest(project_dir: Path, iterations: int = 3) -> float:
    """Benchmark pytest execution."""
    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        result = subprocess.run(
            ["pytest", "-q", str(project_dir)],
            capture_output=True,
            timeout=60,
            cwd=str(project_dir)
        )
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    return avg * 1000  # ms


def benchmark_pytest_parallel(project_dir: Path, iterations: int = 3, workers: int = 4) -> float:
    """Benchmark pytest with parallel execution."""
    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        result = subprocess.run(
            ["pytest", "-n", str(workers), "-q", str(project_dir)],
            capture_output=True,
            timeout=60,
            cwd=str(project_dir)
        )
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    avg = sum(times) / len(times)
    return avg * 1000  # ms


def get_test_count(project_dir: Path) -> int:
    """Get number of tests in project."""
    result = subprocess.run(
        ["pytest", "--collect-only", "-q", str(project_dir)],
        capture_output=True,
        timeout=30
    )
    output = result.stdout.decode()
    # Last line shows count
    lines = [l.strip() for l in output.split('\n') if l.strip()]
    if lines:
        # Try to extract number from last line like "60 tests collected"
        last = lines[-1]
        try:
            return int(last.split()[0])
        except:
            pass
    return 60  # default


def create_realistic_test_project(num_tests: int = 50) -> Path:
    """Create test project with realistic tests (math, string ops, JSON)."""
    tmpdir = Path(tempfile.mkdtemp())

    modules = {
        "test_math.py": """
import math
""" + "\n".join([f"""
def test_math_{i}():
    result = sum(range(1000))
    assert result == 499500
""" for i in range(num_tests // 4)]),
        "test_string.py": """
""" + "\n".join([f"""
def test_string_{i}():
    s = 'test' * {i+1}
    assert len(s) == {4*(i+1)}
""" for i in range(num_tests // 4)]),
        "test_json.py": """
import json
""" + "\n".join([f"""
def test_json_{i}():
    data = json.dumps({{'key': 'value{i}'}})
    assert 'key' in data
""" for i in range(num_tests // 4)]),
        "test_list.py": """
""" + "\n".join([f"""
def test_list_{i}():
    lst = list(range({(i+1)*100}))
    assert len(lst) == {(i+1)*100}
""" for i in range(num_tests // 4)]),
    }

    for filename, content in modules.items():
        (tmpdir / filename).write_text(content)

    return tmpdir


if __name__ == "__main__":
    print("=" * 90)
    print("BENCHMARK 1: NOOP TESTS (minimal overhead measurement)")
    print("=" * 90)

    # Create noop test project
    noop_dir = create_noop_test_project(60)
    noop_count = get_test_count(noop_dir)

    print(f"\nTest Project: {noop_count} noop tests (just `pass` statements)")
    print(f"Location: {noop_dir}\n")

    print("Running benchmarks (3 iterations each)...\n")

    noop_taut_ppe = benchmark_taut_process_per_test(noop_dir)
    noop_taut_ppr = benchmark_taut_process_per_run(noop_dir)
    noop_pytest = benchmark_pytest(noop_dir)

    print(f"Taut (process-per-test):    {noop_taut_ppe:7.1f} ms  ({noop_taut_ppe/noop_count:.2f} ms/test)")
    print(f"Taut (process-per-run):     {noop_taut_ppr:7.1f} ms  ({noop_taut_ppr/noop_count:.2f} ms/test)")
    print(f"Pytest (sequential):        {noop_pytest:7.1f} ms  ({noop_pytest/noop_count:.2f} ms/test)")

    try:
        noop_pytest_parallel = benchmark_pytest_parallel(noop_dir, iterations=2)
        print(f"Pytest (4-worker parallel): {noop_pytest_parallel:7.1f} ms  ({noop_pytest_parallel/noop_count:.2f} ms/test)")
    except Exception as e:
        noop_pytest_parallel = None

    print("\n" + "=" * 90)
    print("BENCHMARK 2: REALISTIC TESTS (with actual work)")
    print("=" * 90)

    # Create realistic test project
    realistic_dir = create_realistic_test_project(50)
    realistic_count = get_test_count(realistic_dir)

    print(f"\nTest Project: {realistic_count} realistic tests (math, string ops, JSON parsing)")
    print(f"Location: {realistic_dir}\n")

    print("Running benchmarks (2 iterations each, these take longer)...\n")

    real_taut_ppe = benchmark_taut_process_per_test(realistic_dir, iterations=2)
    real_taut_ppr = benchmark_taut_process_per_run(realistic_dir, iterations=2)
    real_pytest = benchmark_pytest(realistic_dir, iterations=2)

    print(f"Taut (process-per-test):    {real_taut_ppe:7.1f} ms  ({real_taut_ppe/realistic_count:.2f} ms/test)")
    print(f"Taut (process-per-run):     {real_taut_ppr:7.1f} ms  ({real_taut_ppr/realistic_count:.2f} ms/test)")
    print(f"Pytest (sequential):        {real_pytest:7.1f} ms  ({real_pytest/realistic_count:.2f} ms/test)")

    try:
        real_pytest_parallel = benchmark_pytest_parallel(realistic_dir, iterations=1)
        print(f"Pytest (4-worker parallel): {real_pytest_parallel:7.1f} ms  ({real_pytest_parallel/realistic_count:.2f} ms/test)")
    except Exception as e:
        real_pytest_parallel = None

    print("\n" + "=" * 90)
    print("SUMMARY")
    print("=" * 90)

    print(f"\nNOOP TESTS (minimal work, overhead measurement):")
    print(f"  Taut worker pool: {noop_taut_ppr:.1f}ms total, {noop_taut_ppr/noop_count:.2f}ms/test")
    print(f"  Pytest:          {noop_pytest:.1f}ms total, {noop_pytest/noop_count:.2f}ms/test")
    print(f"  Speedup:         {noop_pytest/noop_taut_ppr:.1f}x")

    print(f"\nREALISTIC TESTS (with computations):")
    print(f"  Taut worker pool: {real_taut_ppr:.1f}ms total, {real_taut_ppr/realistic_count:.2f}ms/test")
    print(f"  Pytest:          {real_pytest:.1f}ms total, {real_pytest/realistic_count:.2f}ms/test")
    print(f"  Speedup:         {real_pytest/real_taut_ppr:.1f}x")

    print(f"\nOVERHEAD ANALYSIS:")
    print(f"  Taut noop overhead:       {noop_taut_ppr/noop_count:.2f}ms/test")
    print(f"  Taut realistic overhead:  {real_taut_ppr/realistic_count:.2f}ms/test")
    print(f"  Consistency:              {abs((noop_taut_ppr/noop_count) - (real_taut_ppr/realistic_count)):.2f}ms variance")

    print(f"\nWORKER POOL EFFICIENCY:")
    print(f"  Process-per-test: {noop_taut_ppe/noop_count:.2f}ms/test (per-process startup)")
    print(f"  Process-per-run:  {noop_taut_ppr/noop_count:.2f}ms/test (worker reuse)")
    print(f"  Efficiency gain:  {noop_taut_ppe/noop_taut_ppr:.1f}x")

    print("\n" + "=" * 90)
