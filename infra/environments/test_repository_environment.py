"""Regressions for the manifest rules and isolated repository cache."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

module_path = Path(__file__).with_name("repository-environment.py")
spec = importlib.util.spec_from_file_location("repository_environment", module_path)
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class RepositoryEnvironmentTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.checkout = Path(temporary.name) / "checkout"
        self.tooling = Path(temporary.name) / "tooling"
        self.checkout.mkdir()
        self.tooling.mkdir()

    def load(self):
        return runtime.Repository(self.checkout, self.tooling)

    def write(self, path, content):
        (self.checkout / path).write_text(content)

    def test_read_only_agent_and_check_select_same_environment(self):
        self.write(".node-version", "v22.16.0\n")
        self.write("package.json", '{"packageManager":"pnpm@10.0.0+sha512.fixture"}')
        self.write("pnpm-lock.yaml", "lockfileVersion: 9\n")
        self.write("Cargo.toml", "[workspace]\n")
        self.write("Cargo.lock", "version = 4\n")
        self.write("rust-toolchain.toml", '[toolchain]\nchannel = "1.90.0"\ncomponents = ["rustfmt"]\n')
        repository = self.load()
        self.assertEqual(repository.tools, {"rust": "1.90.0", "node": "22.16.0", "pnpm": "10.0.0"})
        self.assertEqual(repository.node_manager, "pnpm")
        self.assertTrue(repository.cargo)
        self.write("source.js", "// Ordinary source edits do not invalidate prepared dependencies.\n")
        self.assertEqual(self.load().root, repository.root)
        self.write("pnpm-lock.yaml", "lockfileVersion: 9\nsettings: {}\n")
        self.assertNotEqual(self.load().root, repository.root)

    def test_configuration_rejects_plugin_backends_and_option_injection(self):
        for value in ('"cargo:custom" = "1.0.0"', 'java = "--help"', 'node = "https://example.org/22"'):
            with self.subTest(value=value):
                self.write(".factory-tools.toml", f"[tools]\n{value}\n")
                with self.assertRaises(runtime.PreparationError):
                    self.load()

    def test_failed_preparation_can_be_retried_without_reusing_partial_data(self):
        self.write(".node-version", "22.16.0")
        repository = self.load()
        environment = {"HOME": str(self.tooling), "PATH": runtime.SYSTEM_PATH}

        def interrupted(repo, *_):
            (repo.root / "partial").touch()
            raise runtime.PreparationError("node")

        with patch.object(runtime, "install_tool", side_effect=interrupted):
            with self.assertRaises(runtime.PreparationError):
                runtime.prepare(repository, environment)
        self.assertFalse(repository.root.exists())
        with patch.object(runtime, "install_tool") as install:
            runtime.prepare(repository, environment)
            runtime.prepare(repository, environment)
            self.assertEqual(install.call_count, 1)
        self.assertTrue((repository.root / "ready").is_file())


if __name__ == "__main__":
    unittest.main()
