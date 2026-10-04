"""Support ``python -m taut`` without adding Python startup to ``taut``."""


def main() -> int:
    import os
    import sys
    from importlib.metadata import PackageNotFoundError, distribution

    executable_name = "taut.exe" if os.name == "nt" else "taut"
    try:
        package = distribution("taut")
    except PackageNotFoundError:
        raise SystemExit("taut is not installed. Run `python -m pip install taut`.") from None

    # RECORD points to the binary installed alongside this distribution. Looking
    # it up here also handles --user, --prefix, virtualenv and Windows installs.
    executable = next(
        (
            package.locate_file(path)
            for path in package.files or ()
            if path.name == executable_name and package.locate_file(path).is_file()
        ),
        None,
    )
    if executable is None:
        raise SystemExit(
            "The taut executable is missing. Reinstall taut with "
            "`python -m pip install --force-reinstall taut`."
        )

    environment = os.environ.copy()
    environment.setdefault("TAUT_PYTHON", sys.executable)
    args = [str(executable), *sys.argv[1:]]
    if os.name == "nt":
        import subprocess

        return subprocess.call(args, env=environment)

    os.execve(executable, args, environment)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
