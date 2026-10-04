"""The helper preserves callables and keeps parameter metadata dependency-free."""
import importlib.util
from pathlib import Path
import sys
import unittest


def load_helper():
    spec = importlib.util.spec_from_file_location(
        "standalone_taut_parametrize", Path(__file__).parents[1] / "python/taut/parametrize.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.parametrize


class TestParametrizeHelper(unittest.TestCase):
    def test_preserves_function_identity_and_signature(self):
        parametrize = load_helper()

        async def original(value):
            return value

        decorated = parametrize("value", [1, 2], ids=["one", "two"])(original)
        self.assertIs(decorated, original)
        self.assertEqual(original._taut_parametrize[0]["argvalues"], [1, 2])

    def test_stacked_metadata_is_immutable_and_applies_bottom_first(self):
        parametrize = load_helper()

        def function(x, y):
            return x + y

        parametrize("y", [1, 2])(function)
        first = function._taut_parametrize
        parametrize("x", [3, 4])(function)
        self.assertEqual(len(first), 1)
        self.assertEqual([spec["argnames"] for spec in function._taut_parametrize], ["y", "x"])
        self.assertEqual(function(1, 2), 3)

    def test_does_not_import_pytest_or_native_extension(self):
        before = set(sys.modules)
        load_helper()
        imported = set(sys.modules) - before
        self.assertFalse(any(name.startswith("pytest") or name == "taut._taut" for name in imported))

    def test_rejects_indirect_parameters(self):
        with self.assertRaisesRegex(ValueError, "indirect"):
            load_helper()("value", [1], indirect=True)


if __name__ == "__main__":
    unittest.main()
