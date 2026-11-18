"""The native provider cannot begin its handshake before repository activation."""

import contextlib
import errno
import importlib.util
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

module_path = Path(__file__).parents[2] / "crates/adapters/src/environments/feature_execution/activation.py"
spec = importlib.util.spec_from_file_location("activation", module_path)
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class PreparationError(Exception):
    pass


class Executed(Exception):
    pass


class ActivationTest(unittest.TestCase):
    def runtime(self):
        return SimpleNamespace(
            CHECKOUT=Path("/checkout"), TOOLING=Path("/tooling"),
            Repository=Mock(return_value="repository"),
            activate=Mock(return_value={"PATH": "prepared-toolchain"}),
            PreparationError=PreparationError,
        )

    def test_readiness_follows_activation_and_precedes_the_unchanged_native_command(self):
        for access, writable in [("read", False), ("write", True)]:
            with self.subTest(access=access):
                runtime = self.runtime()
                output = io.StringIO()

                def activated(*args):
                    self.assertEqual(output.getvalue(), "")
                    return {"PATH": "prepared-toolchain"}

                def executed(*args):
                    self.assertEqual(args, ("/usr/bin/native", ["/usr/bin/native", "--stdio"],
                                            {"PATH": "prepared-toolchain"}))
                    self.assertEqual(json.loads(output.getvalue()), {"type": "factory_repository_ready"})
                    raise Executed()

                runtime.activate.side_effect = activated
                with patch.object(bootstrap, "load_runtime", return_value=runtime), \
                        patch.object(bootstrap.os, "execve", side_effect=executed), \
                        contextlib.redirect_stdout(output):
                    with self.assertRaises(Executed):
                        bootstrap.run(access, ["/usr/bin/native", "--stdio"])
                self.assertEqual(runtime.activate.call_args.args[:2], ("repository", writable))

    def test_failed_preparation_never_executes_a_provider_or_exposes_private_output(self):
        for error, step in [
            (PreparationError("node-offline-install"), "node-offline-install"),
            (PreparationError("private repository URL"), "configuration"),
            (OSError(errno.ENOSPC, "private path and credential"), "disk-space"),
        ]:
            with self.subTest(step=step):
                runtime = self.runtime()
                runtime.activate.side_effect = error
                output = io.StringIO()
                with patch.object(bootstrap, "load_runtime", return_value=runtime), \
                        patch.object(bootstrap.os, "execve") as execute, \
                        contextlib.redirect_stdout(output):
                    self.assertEqual(bootstrap.run("write", ["/usr/bin/native"]), 1)
                execute.assert_not_called()
                self.assertEqual(json.loads(output.getvalue()),
                                 {"type": "factory_repository_failed", "step": step})


if __name__ == "__main__":
    unittest.main()
