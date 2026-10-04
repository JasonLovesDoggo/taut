#!/usr/bin/env python3
"""Reproducible, validated end-to-end benchmarks for taut and pytest."""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import random
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

WORKLOADS = ("noop", "cpu", "blocking", "threaded", "async", "collection")
ANSI = re.compile(r"\x1b\[[0-9;]*m")

# No per-test file I/O: completion witnesses are appended in memory and written
# once when each Python process exits. Both runners execute this same code.
PROBE = '''import atexit
import json
import os
from pathlib import Path
import sys

_completed = []
completed = _completed.append

def parallel(function):
    return function

def _flush():
    directory = Path(os.environ["TAUT_BENCH_AUDIT"])
    record = {"tests": _completed, "python": sys.executable, "prefix": sys.prefix}
    (directory / (str(os.getpid()) + ".json")).write_text(json.dumps(record))

atexit.register(_flush)
'''


class BenchmarkError(RuntimeError):
    """A sample is invalid and must never contribute a timing."""


@dataclass(frozen=True)
class Runner:
    name: str
    kind: str
    command: tuple[str, ...]


def positive_int(value: str) -> int:
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return result


def nonnegative_int(value: str) -> int:
    result = int(value)
    if result < 0:
        raise argparse.ArgumentTypeError("must not be negative")
    return result


def positive_float(value: str) -> float:
    result = float(value)
    if not 0 < result < float("inf"):
        raise argparse.ArgumentTypeError("must be positive and finite")
    return result


def make_project(directory: Path, workload: str, count: int, files: int,
                 sleep: float, cpu_iterations: int) -> list[str]:
    """Generate exactly count unique tests, including functions and methods."""
    directory.mkdir(parents=True)
    (directory / "bench_probe.py").write_text(PROBE)
    (directory / "pytest.ini").write_text("[pytest]\n")
    expected = []
    cpu_result = sum(x * x % 97 for x in range(cpu_iterations)) if workload == "cpu" else 0
    for part in range(min(files, count)):
        filename = f"test_part_{part:03d}.py"
        imports = ["from bench_probe import completed, parallel"]
        if workload in ("blocking", "threaded"):
            imports.append("import time")
        if workload == "threaded":
            imports.append("from concurrent.futures import ThreadPoolExecutor")
        if workload == "async":
            imports.append("import asyncio")
        lines = [*imports, ""]
        # Alternate modules of functions/methods to exercise both collection paths.
        is_class = part % 2 == 1
        indent = "    " if is_class else ""
        if is_class:
            lines.append("class TestWorkload:")
        for index in range(part, count, min(files, count)):
            name = f"test_{index:05d}"
            test_id = filename + ("::TestWorkload" if is_class else "") + "::" + name
            expected.append(test_id)
            declaration = "async def" if workload == "async" else "def"
            lines.extend([f"{indent}@parallel",
                          f"{indent}{declaration} {name}({'self' if is_class else ''}):"])
            body = []
            if workload == "cpu":
                body = ["total = 0", f"for value in range({cpu_iterations}):",
                        "    total += value * value % 97",
                        f"assert total == {cpu_result}"]
            elif workload == "blocking":
                body = [f"time.sleep({sleep!r})"]
            elif workload == "threaded":
                body = ["with ThreadPoolExecutor(max_workers=2) as pool:",
                        f"    futures = [pool.submit(time.sleep, {sleep!r}) for _ in range(2)]",
                        "    for future in futures:", "        assert future.result() is None"]
            elif workload == "async":
                body = [f"await asyncio.sleep({sleep!r})"]
            body.append(f"completed({test_id!r})")
            lines.extend(indent + "    " + line for line in body)
            lines.append("")
        (directory / filename).write_text("\n".join(lines) + "\n")
    return sorted(expected)


def run_command(command: list[str], cwd: Path, env: dict[str, str],
                timeout: float) -> tuple[float, subprocess.CompletedProcess]:
    start = time.perf_counter()
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True,
                               start_new_session=os.name == "posix")
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        # A timed-out runner must not leave Python workers competing with later samples.
        terminate(process)
        raise BenchmarkError(f"timed out after {timeout}s: {shlex.join(command)}") from error
    except KeyboardInterrupt:
        terminate(process)
        raise
    elapsed = time.perf_counter() - start
    result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    if result.returncode:
        raise BenchmarkError(f"exit {result.returncode}: {shlex.join(command)}\n"
                             f"{stdout}\n{stderr}")
    return elapsed, result


def terminate(process: subprocess.Popen) -> None:
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
    except ProcessLookupError:
        pass
    process.communicate()


def validate_sample(result: subprocess.CompletedProcess, expected: list[str],
                    audit: Path, collection: bool, kind: str) -> dict:
    if result.returncode:
        raise BenchmarkError(f"runner exited with {result.returncode}")
    output = ANSI.sub("", result.stdout)
    if collection:
        pattern = r"(?m)^\s*(\d+) tests?\s*$" if kind == "taut" else r"\b(\d+) tests? collected\b"
        observed = []
        for line in output.splitlines():
            if "::test_" in line or "::TestWorkload::test_" in line:
                pieces = line.strip().split("::")
                observed.append("::".join([Path(pieces[0]).name, *pieces[1:]]))
        if Counter(observed) != Counter(expected):
            raise BenchmarkError(f"collection IDs differ: expected {len(expected)}, got {len(observed)}\n{output}")
    else:
        pattern = r"\b(\d+) passed\b"
        # Cached/skipped results, even if the runner also reports passes, are invalid.
        if re.search(r"\b[1-9]\d* (?:failed|skipped|errors?|xfailed|xpassed|deselected)\b", output):
            raise BenchmarkError(f"non-passing test results in sample:\n{output}")
    counts = [int(value) for value in re.findall(pattern, output)]
    if counts != [len(expected)]:
        raise BenchmarkError(f"expected one exact {'collection' if collection else 'pass'} count "
                             f"of {len(expected)}, got {counts}\n{output}")
    observed = []
    processes = 0
    for witness in audit.glob("*.json"):
        record = json.loads(witness.read_text())
        if Path(record["prefix"]).resolve() != Path(sys.prefix).resolve():
            raise BenchmarkError(f"runner used a different Python environment: {record['prefix']}")
        if Path(record["python"]).resolve() != Path(sys.executable).resolve():
            raise BenchmarkError(f"runner used a different Python interpreter: {record['python']}")
        observed.extend(record["tests"])
        processes += bool(record["tests"])
    target = [] if collection else expected
    if Counter(observed) != Counter(target):
        missing = sorted((Counter(target) - Counter(observed)).elements())[:10]
        extra = sorted((Counter(observed) - Counter(target)).elements())[:10]
        raise BenchmarkError(f"completion audit failed: missing={missing}, extra={extra}")
    return {"reported_count": len(expected), "completed_count": len(observed),
            "test_processes": processes, "returncode": result.returncode}


def build_runners(binaries: list[tuple[str, Path, list[str]]], workload: str,
                  jobs: int, skip_pytest: bool, skip_xdist: bool) -> list[Runner]:
    runners = []
    for label, binary, extra in binaries:
        if workload == "collection":
            runners.append(Runner(label, "taut", (str(binary), "list", ".")))
        else:
            for workers in dict.fromkeys([1, jobs]):
                command = (str(binary), ".", "--no-cache", "-j", str(workers), *extra)
                runners.append(Runner(f"{label}-j{workers}", "taut", command))
    if not skip_pytest:
        command = (sys.executable, "-m", "pytest", "-q", "--tb=short", "-p", "no:cacheprovider")
        if workload == "async":
            command += ("-p", "pytest_asyncio.plugin", "--asyncio-mode=auto")
        if workload == "collection":
            runners.append(Runner("pytest", "pytest", command + ("--collect-only", ".")))
        else:
            runners.append(Runner("pytest", "pytest", command + (".",)))
            if not skip_xdist:
                runners.append(Runner(f"pytest-xdist-j{jobs}", "pytest",
                                      command + ("-p", "xdist.plugin", "-n", str(jobs), ".")))
    return runners


def binary_metadata(path: Path) -> dict:
    version = subprocess.run([str(path), "--version"], check=True, capture_output=True,
                             text=True, timeout=10).stdout.strip()
    return {"path": str(path), "version": version,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--taut", type=Path, action="append", help="release binary; repeat to compare candidates")
    parser.add_argument("--baseline", type=Path, help="optional previous release binary")
    parser.add_argument("--taut-arg", action="append", default=[], help="extra candidate argument; use --taut-arg=--flag=value")
    parser.add_argument("--baseline-arg", action="append", default=[])
    parser.add_argument("--workload", choices=WORKLOADS, action="append", help="repeat to select workloads; default: all")
    parser.add_argument("--tests", type=positive_int, default=128)
    parser.add_argument("--files", type=positive_int, default=16)
    parser.add_argument("--jobs", type=positive_int, default=min(4, os.cpu_count() or 1))
    parser.add_argument("--warmups", type=nonnegative_int, default=1)
    parser.add_argument("--repeats", type=positive_int, default=5)
    parser.add_argument("--sleep", type=positive_float, default=0.02, help="seconds for blocking/threaded/async waits")
    parser.add_argument("--cpu-iterations", type=positive_int, default=200_000)
    parser.add_argument("--timeout", type=positive_float, default=120)
    parser.add_argument("--seed", type=int, default=0, help="reproducible interleaved sample order")
    parser.add_argument("--skip-pytest", action="store_true")
    parser.add_argument("--skip-xdist", action="store_true")
    parser.add_argument("--output", type=Path, default=Path("benchmark-results.json"))
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    paths = args.taut or [Path("target/release/taut")]
    binaries = [("taut" if len(paths) == 1 else f"taut-{index + 1}", path.resolve(), args.taut_arg)
                for index, path in enumerate(paths)]
    if args.baseline:
        binaries.append(("baseline", args.baseline.resolve(), args.baseline_arg))
    for _, path, _ in binaries:
        if not path.is_file() or not os.access(path, os.X_OK):
            raise BenchmarkError(f"not an executable: {path}; build with cargo build --release")
        if "debug" in path.parts:
            raise BenchmarkError(f"debug build is unsuitable for comparisons: {path}")
    env = os.environ.copy()
    # Use the harness's venv for every runner and keep unrelated plugins/config out.
    env.update(PATH=str(Path(sys.executable).parent) + os.pathsep + env.get("PATH", ""),
               PYTEST_DISABLE_PLUGIN_AUTOLOAD="1", PYTEST_ADDOPTS="", PYTHONHASHSEED="0",
               NO_COLOR="1", CLICOLOR="0", RAYON_NUM_THREADS=str(args.jobs),
               VIRTUAL_ENV=sys.prefix)
    env.pop("PYTHONPATH", None)
    env.pop("PYTEST_PLUGINS", None)
    env.pop("TAUT_PYTHON", None)
    dependencies = {}
    workloads = list(dict.fromkeys(args.workload or WORKLOADS))
    required = [] if args.skip_pytest else ["pytest"]
    if not args.skip_pytest and "async" in workloads:
        required.append("pytest-asyncio")
    if not args.skip_pytest and not args.skip_xdist and any(name != "collection" for name in workloads):
        required.append("pytest-xdist")
    for package in required:
        try:
            dependencies[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError as error:
            raise BenchmarkError(f"missing {package}; install benchmark dependencies in this Python environment") from error
    report = {"schema_version": 1, "created_at": datetime.now(timezone.utc).isoformat(),
              "platform": platform.platform(), "machine": platform.machine(),
              "cpu_count": os.cpu_count(), "python": sys.version, "python_executable": sys.executable,
              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "python_prefix": sys.prefix, "dependencies": dependencies,
              "config": {key: value for key, value in vars(args).items()
                         if key not in ("taut", "baseline", "output")},
              "binaries": {label: binary_metadata(path) for label, path, _ in binaries},
              "environment": {key: env.get(key) for key in ("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "PYTEST_ADDOPTS",
                              "PYTHONHASHSEED", "NO_COLOR", "RAYON_NUM_THREADS")},
              "methodology": "Fresh CLI process per sample; warm filesystem/import caches; no incremental test skipping; "
                             "in-memory completion witnesses flushed at worker exit; all samples validated; "
                             "randomized runner order each round; startup and shutdown included.",
              "fixtures": {}, "results": [], "status": "running"}
    rng = random.Random(args.seed)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix="taut-benchmark-") as temporary:
            root = Path(temporary)
            for workload in workloads:
                project = root / workload
                expected = make_project(project, workload, args.tests, args.files, args.sleep, args.cpu_iterations)
                report["fixtures"][workload] = {
                    "expected_ids": expected,
                    "source_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                      for path in sorted(project.glob("*.py"))},
                }
                runners = build_runners(binaries, workload, args.jobs, args.skip_pytest, args.skip_xdist)
                entries = {}
                for runner in runners:
                    entry = {"workload": workload, "runner": runner.name, "command": list(runner.command),
                             "cwd": str(project), "expected_count": len(expected), "samples": [], "warmups": []}
                    report["results"].append(entry)
                    entries[runner.name] = entry
                for round_index in range(args.warmups + args.repeats):
                    order = runners.copy()
                    rng.shuffle(order)
                    for runner in order:
                        audit = root / "audit"
                        shutil.rmtree(audit, ignore_errors=True)
                        audit.mkdir()
                        sample_env = dict(env, TAUT_BENCH_AUDIT=str(audit), PYTHONPATH=str(project))
                        elapsed, result = run_command(list(runner.command), project, sample_env, args.timeout)
                        validated = validate_sample(result, expected, audit, workload == "collection", runner.kind)
                        sample = {"seconds": elapsed, "round": round_index, **validated}
                        group = "warmups" if round_index < args.warmups else "samples"
                        entries[runner.name][group].append(sample)
                for entry in entries.values():
                    timings = [sample["seconds"] for sample in entry["samples"]]
                    entry["median_seconds"] = statistics.median(timings)
                    entry["min_seconds"] = min(timings)
                    entry["max_seconds"] = max(timings)
                    print(f"{workload:11} {entry['runner']:21} {statistics.median(timings) * 1000:9.2f} ms "
                          f"[{min(timings) * 1000:.2f}, {max(timings) * 1000:.2f}]  n={len(timings)}", flush=True)
            report["status"] = "complete"
    except (BenchmarkError, OSError, ValueError, KeyboardInterrupt) as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"Results: {args.output.resolve()}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BenchmarkError, OSError, subprocess.SubprocessError) as error:
        print(f"Benchmark invalid: {error}", file=sys.stderr)
        raise SystemExit(1)
