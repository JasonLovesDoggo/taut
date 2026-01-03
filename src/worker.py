import sys
import struct
import traceback
import importlib.util
import inspect
import asyncio
import io
import contextlib
import os
import time
import msgpack


def _run_maybe_async(callable_obj):
    result = callable_obj()
    if inspect.isawaitable(result):
        asyncio.run(result)


def _should_track(filename):
    if not filename or filename.startswith("<"):
        return False
    return not any(x in filename for x in ["site-packages", "lib/python", "/usr/lib"])


def _collect_coverage_with_settrace():
    executed_lines = {}

    def trace_function(frame, event, arg):
        if event == "line":
            filename = frame.f_code.co_filename
            if _should_track(filename):
                abs_path = os.path.abspath(filename)
                executed_lines.setdefault(abs_path, set()).add(frame.f_lineno)
        return trace_function

    return executed_lines, trace_function


def _collect_coverage_with_monitoring():
    mon = sys.monitoring
    executed_lines = {}
    seen_code = set()

    def on_call(code, instruction_offset):
        filename = getattr(code, "co_filename", "")
        if not _should_track(filename):
            return
        if code in seen_code:
            return
        seen_code.add(code)
        mon.set_local_events(tool_id, code, mon.events.LINE)

    def on_line(code, line_number):
        filename = getattr(code, "co_filename", "")
        if not _should_track(filename):
            return
        abs_path = os.path.abspath(filename)
        executed_lines.setdefault(abs_path, set()).add(line_number)

    tool_id = None
    for tid in range(1, mon.MAX_TOOL_ID + 1):
        try:
            mon.use_tool_id(tid, "taut_worker")
        except ValueError:
            continue
        tool_id = tid
        break

    if tool_id is None:
        raise RuntimeError("No free sys.monitoring tool id")

    mon.register_callback(tool_id, mon.events.CALL, on_call)
    mon.register_callback(tool_id, mon.events.LINE, on_line)
    mon.set_events(tool_id, mon.events.CALL)

    def uninstall():
        mon.set_events(tool_id, 0)
        mon.register_callback(tool_id, mon.events.CALL, None)
        mon.register_callback(tool_id, mon.events.LINE, None)
        mon.free_tool_id(tool_id)

    return executed_lines, uninstall


def run_test(req):
    test_file = req["file"]
    test_name = req["function"]
    class_name = req.get("class")
    collect_coverage = req.get("collect_coverage", False)
    request_id = req.get("id", 0)

    result = {
        "id": request_id,
        "passed": False,
        "error": None,
        "stdout": "",
        "stderr": "",
        "duration_sec": 0.0,
    }

    executed_lines = None
    uninstall = None
    trace_fn = None

    start = time.perf_counter()

    try:
        test_dir = os.path.dirname(os.path.abspath(test_file))
        if test_dir not in sys.path:
            sys.path.insert(0, test_dir)

        if collect_coverage:
            try:
                executed_lines, uninstall = _collect_coverage_with_monitoring()
            except Exception:
                executed_lines, trace_fn = _collect_coverage_with_settrace()
                sys.settrace(trace_fn)

        out_buf = io.StringIO()
        err_buf = io.StringIO()

        # Use unique module name to avoid cache issues
        mod_name = f"taut_test_{request_id}"

        with contextlib.redirect_stdout(out_buf), contextlib.redirect_stderr(err_buf):
            spec = importlib.util.spec_from_file_location(mod_name, test_file)
            module = importlib.util.module_from_spec(spec)
            sys.modules[mod_name] = module
            spec.loader.exec_module(module)

            if class_name:
                cls = getattr(module, class_name)
                instance = cls()
                try:
                    if hasattr(instance, "setUp"):
                        instance.setUp()
                    test_func = getattr(instance, test_name)
                    _run_maybe_async(test_func)
                    result["passed"] = True
                finally:
                    # Always run tearDown, even if test fails
                    if hasattr(instance, "tearDown"):
                        instance.tearDown()
            else:
                test_func = getattr(module, test_name)
                _run_maybe_async(test_func)
                result["passed"] = True

        # Clean up module from sys.modules
        sys.modules.pop(mod_name, None)

        result["stdout"] = out_buf.getvalue()
        result["stderr"] = err_buf.getvalue()

    except AssertionError as e:
        result["stdout"] = out_buf.getvalue() if 'out_buf' in dir() else ""
        result["stderr"] = err_buf.getvalue() if 'err_buf' in dir() else ""
        result["error"] = {"message": str(e) or "Assertion failed", "traceback": traceback.format_exc()}
    except Exception as e:
        result["stdout"] = out_buf.getvalue() if 'out_buf' in dir() else ""
        result["stderr"] = err_buf.getvalue() if 'err_buf' in dir() else ""
        result["error"] = {"message": f"{type(e).__name__}: {e}", "traceback": traceback.format_exc()}

    finally:
        if trace_fn is not None:
            sys.settrace(None)
        if uninstall is not None:
            try:
                uninstall()
            except Exception:
                pass

        if executed_lines is not None:
            result["coverage"] = {k: sorted(v) for k, v in executed_lines.items()}

        result["duration_sec"] = time.perf_counter() - start

    return result


def _read_message():
    """Read length-prefixed msgpack message from stdin."""
    len_bytes = sys.stdin.buffer.read(4)
    if not len_bytes or len(len_bytes) < 4:
        return None

    length = struct.unpack('<I', len_bytes)[0]
    data = sys.stdin.buffer.read(length)
    if len(data) < length:
        return None

    return msgpack.unpackb(data, raw=False)

def _send_message(msg):
    """Send length-prefixed msgpack message to stdout."""
    data = msgpack.packb(msg, use_bin_type=True)
    length = struct.pack('<I', len(data))
    sys.stdout.buffer.write(length + data)
    sys.stdout.buffer.flush()

def main():
    while True:
        try:
            req = _read_message()
            if not req:
                break

            if req.get("cmd") == "shutdown":
                break

            if req.get("cmd") == "ping":
                _send_message({"id": req.get("id", 0), "pong": True})
                continue

            resp = run_test(req)

        except Exception as e:
            resp = {
                "id": req.get("id", -1) if isinstance(req, dict) else -1,
                "passed": False,
                "error": {"message": f"Worker error: {e}", "traceback": traceback.format_exc()},
                "stdout": "",
                "stderr": "",
                "duration_sec": 0.0,
            }

        _send_message(resp)


if __name__ == "__main__":
    main()
