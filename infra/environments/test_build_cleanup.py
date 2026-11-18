"""Exercise the embedded worker cleanup against real, disposable Git checkouts."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

module_path = Path(__file__).parents[2] / "crates/adapters/src/environments/feature_execution/build_cleanup.py"
spec = importlib.util.spec_from_file_location("build_cleanup", module_path)
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class BuildCleanupTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.checkout = self.root / "checkout"
        self.checkout.mkdir()
        self.git("init")
        (self.checkout / ".gitignore").write_text("/target/\n/node_modules/\n")
        (self.checkout / "source.rs").write_text("preserve source changes")
        (self.checkout / "package.json").write_text('{"name":"fixture"}')
        self.git("add", "source.rs", ".gitignore")

    def git(self, *args):
        subprocess.run(["/usr/bin/git", "-C", str(self.checkout), *args],
                       capture_output=True, check=True)

    def build(self, name):
        path = self.checkout / name
        path.mkdir()
        (path / "generated.bin").write_bytes(b"regenerable")
        if name == "target":
            (path / "CACHEDIR.TAG").write_text("Signature: 8a477f597d28d172789f06886806bc55\n")
        return path

    def test_removes_only_builds_and_dependencies_and_is_repeatable(self):
        self.build("target")
        self.build("node_modules")
        (self.checkout / "draft.md").write_text("uncommitted draft")
        (self.checkout / "output.pdf").write_bytes(b"user output")
        checkpoint = self.checkout / ".git/refs/factory/attempts"
        checkpoint.mkdir(parents=True)
        (checkpoint / "retained").write_text("a" * 40)
        runtime.cleanup(self.checkout)
        runtime.cleanup(self.checkout)
        self.assertFalse((self.checkout / "target").exists())
        self.assertFalse((self.checkout / "node_modules").exists())
        self.assertEqual((self.checkout / "source.rs").read_text(), "preserve source changes")
        self.assertEqual((self.checkout / "draft.md").read_text(), "uncommitted draft")
        self.assertEqual((self.checkout / "output.pdf").read_bytes(), b"user output")
        self.assertEqual((checkpoint / "retained").read_text(), "a" * 40)

    def test_preserves_tracked_content_and_nonignored_directories(self):
        tracked = self.build("target")
        self.git("add", "-f", "target/generated.bin")
        dependencies = self.build("node_modules")
        (self.checkout / ".gitignore").write_text("/target/\n")
        runtime.cleanup(self.checkout)
        self.assertTrue((tracked / "generated.bin").is_file())
        self.assertTrue((dependencies / "generated.bin").is_file())

    def test_preserves_unrecognized_targets_and_non_node_dependencies(self):
        target = self.build("target")
        (target / "CACHEDIR.TAG").unlink()
        dependencies = self.build("node_modules")
        (self.checkout / "package.json").unlink()
        runtime.cleanup(self.checkout)
        self.assertTrue((target / "generated.bin").is_file())
        self.assertTrue((dependencies / "generated.bin").is_file())

    def test_never_follows_external_links(self):
        outside = self.root / "external"
        outside.mkdir()
        (outside / "keep").write_text("preserve")
        (self.checkout / "target").symlink_to(outside, target_is_directory=True)
        dependencies = self.build("node_modules")
        (dependencies / "outside").symlink_to(outside, target_is_directory=True)
        runtime.cleanup(self.checkout)
        self.assertTrue((self.checkout / "target").is_symlink())
        self.assertFalse(dependencies.exists())
        self.assertEqual((outside / "keep").read_text(), "preserve")

    def test_refuses_unsafe_metadata_or_an_unreadable_index(self):
        target = self.build("target")
        with patch.object(runtime, "git", return_value=subprocess.CompletedProcess([], 128, b"")):
            with self.assertRaises(RuntimeError):
                runtime.cleanup(self.checkout)
        self.assertTrue(target.exists())
        (self.checkout / ".git").rename(self.root / "metadata")
        (self.checkout / ".git").symlink_to(self.root / "metadata", target_is_directory=True)
        with self.assertRaises(OSError):
            runtime.cleanup(self.checkout)
        self.assertTrue(target.exists())


if __name__ == "__main__":
    unittest.main()
