#!/usr/bin/env python3
"""Compatibility entry point for measured, validated collection comparisons."""

import subprocess
import sys
from compare_execution import BenchmarkError, main

if __name__ == "__main__":
    try:
        raise SystemExit(main(["--workload", "collection", *sys.argv[1:]]))
    except (BenchmarkError, OSError, subprocess.SubprocessError) as error:
        print(f"Benchmark invalid: {error}", file=sys.stderr)
        raise SystemExit(1)
