"""Exercise only the installed wheel, outside the source tree."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import taut


assert "taut._taut" not in sys.modules, "Decorators must not load a native extension"
assert "typing" not in sys.modules, "Decorator imports should stay lightweight"


@taut.mark(group="packaging")
@taut.parallel
@taut.skip("example")
def decorated():
    pass


assert decorated._taut_markers == {"group": "packaging"}
assert decorated._taut_parallel
assert decorated._taut_skip_reason == "example"

binary = shutil.which("taut")
assert binary, "The wheel must install a taut executable"
with open(binary, "rb") as executable:
    assert executable.read(2) != b"#!", "taut must start as a native binary"

environment = os.environ.copy()
# Force interpreter selection to use the installation or python -m. Neither the
# checkout nor a developer's environment should make an incomplete wheel pass.
for name in ("PYTHONPATH", "TAUT_PYTHON", "VIRTUAL_ENV"):
    environment.pop(name, None)
environment["PATH"] = os.defpath

with tempfile.TemporaryDirectory(prefix="taut-wheel-") as directory:
    test_file = Path(directory, "test_installed.py")
    test_file.write_text(
        "import asyncio\n"
        "import sys\n"
        "from taut import parallel\n"
        f"EXPECTED_PREFIX = {sys.prefix!r}\n"
        "def test_interpreter():\n"
        "    assert sys.prefix == EXPECTED_PREFIX\n"
        "@parallel\n"
        "async def test_async():\n"
        "    await asyncio.sleep(0)\n"
        "    assert sys.prefix == EXPECTED_PREFIX\n"
    )
    for command in ([binary], [sys.executable, "-m", "taut"]):
        subprocess.run([*command, "--help"], cwd=directory, env=environment, check=True)
        for isolation in ("process-per-run", "process-per-test"):
            subprocess.run(
                [*command, "--no-cache", "--isolation", isolation, str(test_file)],
                cwd=directory,
                env=environment,
                check=True,
            )
        failing = Path(directory, "test_failure.py")
        failing.write_text("def test_failure():\n    assert False, 'intentional'\n")
        result = subprocess.run(
            [*command, "--no-cache", str(failing)], cwd=directory, env=environment
        )
        assert result.returncode == 1, result.returncode

print("Installed wheel smoke tests passed")
