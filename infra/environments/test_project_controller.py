#!/usr/bin/python3
"""Local synthetic filesystem and Git regressions; no real accounts or network."""
import importlib.util
import base64
import contextlib
from concurrent.futures import ThreadPoolExecutor
import fcntl
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import threading
import unittest
import uuid
from unittest.mock import patch

MODULE_PATH = Path(__file__).with_name('project-controller.py')
spec = importlib.util.spec_from_file_location('factory_project_controller', MODULE_PATH)
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


class ProjectControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='factory-project-test-')
        self.home = Path(self.tmp.name)
        (self.home / 'Documents').mkdir()
        self.project = str(uuid.uuid4())
        self.operation = str(uuid.uuid4())

    def tearDown(self):
        self.tmp.cleanup()

    def execute(self, op, **fields):
        return controller.execute({'op': op, **fields}, str(self.home))

    def origin(self):
        repo = self.home / 'origin'
        repo.mkdir()
        subprocess.run(['git', 'init', '-q', str(repo)], check=True)
        subprocess.run(['git', '-C', str(repo), 'config', 'user.name', 'Synthetic User'], check=True)
        subprocess.run(['git', '-C', str(repo), 'config', 'user.email', 'fixture@example.invalid'], check=True)
        (repo / 'file.txt').write_text('sample\n')
        subprocess.run(['git', '-C', str(repo), 'add', 'file.txt'], check=True)
        subprocess.run(['git', '-C', str(repo), 'commit', '-q', '-m', 'fixture'], check=True)
        return repo

    def clone(self, origin):
        with patch.object(controller, 'repository', return_value=str(origin)):
            return self.execute('clone', project=self.project, operation=self.operation, repository='owner/repo', directory='repo')

    def inspect(self):
        return self.execute('inspect', project=self.project, operation=self.operation, directory='repo')

    def feature(self, origin, feature, op, **fields):
        with patch.object(controller, 'repository', return_value=str(origin)):
            common = dict(project=self.project, operation=self.operation, directory='repo',
                          repository='owner/repo', repository_id=123, feature=feature)
            if op == 'initialize':
                candidate = str(uuid.uuid4())
                branch = self.execute('feature_stage_initialization', **common, candidate=candidate)
                return self.execute('feature_confirm_initialization', **common, candidate=candidate,
                                    expected_branch=branch)
            if op == 'refresh':
                candidate = str(uuid.uuid4())
                proposed = self.execute('feature_stage_refresh', **common, **fields, candidate=candidate)
                if proposed == fields:
                    return self.execute('feature_snapshot', **common)
                return self.execute('feature_confirm_refresh', **common, **fields, candidate=candidate,
                                    proposed_snapshot=proposed)
            return self.execute('feature_' + op, **common, **fields)

    def feature_checkout(self, feature, attempt):
        return self.home / '.factory/features' / self.project / feature / 'attempts' / attempt / 'checkout'

    def snapshot(self):
        result = {}
        for directory, names, files in os.walk(self.home, followlinks=False):
            for name in sorted(names + files):
                path = Path(directory) / name
                info = path.lstat()
                item = (stat.S_IFMT(info.st_mode), stat.S_IMODE(info.st_mode))
                if path.is_symlink():
                    item += (os.readlink(path),)
                elif path.is_file():
                    item += (path.read_bytes(),)
                result[str(path.relative_to(self.home))] = item
        return result

    def test_setup_status_missing_is_read_only_and_never_generates_key(self):
        before = self.snapshot()
        with patch.object(controller, 'generate_candidate', side_effect=AssertionError('must not generate')):
            with patch.object(controller, 'public_key', side_effect=AssertionError('must not invoke keygen')):
                self.assertFalse(self.execute('setup_status'))
        self.assertEqual(self.snapshot(), before)
        self.assertFalse((self.home / '.factory').exists())

    def test_setup_status_only_reports_registered_key_and_valid_git_identity(self):
        key = self.execute('key')
        before = self.snapshot()
        with patch.object(controller, 'generate_candidate', side_effect=AssertionError('must not generate')):
            with patch.object(controller, 'public_key', side_effect=AssertionError('must not invoke keygen')):
                self.assertFalse(self.execute('setup_status'))
        self.assertEqual(self.snapshot(), before)
        self.execute('mark', expected=key['public_key'])
        self.assertFalse(self.execute('setup_status'))
        self.execute('identity', name='alice', email='alice@example.invalid')
        before = self.snapshot()
        with patch.object(controller, 'generate_candidate', side_effect=AssertionError('must not generate')):
            with patch.object(controller, 'public_key', side_effect=AssertionError('must not invoke keygen')):
                self.assertTrue(self.execute('setup_status'))
        self.assertEqual(self.snapshot(), before)

    def test_setup_status_does_not_claim_ready_during_key_rotation(self):
        key = self.execute('key')
        self.execute('mark', expected=key['public_key'])
        self.execute('identity', name='alice', email='alice@example.invalid')
        before = self.snapshot()
        lock = os.open(str(self.home / '.factory/ssh/lock'), os.O_RDONLY)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertFalse(self.execute('setup_status'))
        finally:
            os.close(lock)
        self.assertEqual(self.snapshot(), before)
        self.assertTrue(self.execute('setup_status'))

    def test_setup_status_rejects_corrupt_receipt_private_key_and_symlinks(self):
        key = self.execute('key')
        self.execute('mark', expected=key['public_key'])
        self.execute('identity', name='alice', email='alice@example.invalid')
        active = self.home / '.factory/ssh/active'
        receipt = active / 'registered'
        original = receipt.read_bytes()
        receipt.write_text('{"sha256":"wrong"}')
        before = self.snapshot()
        with self.assertRaises(controller.Problem) as caught:
            self.execute('setup_status')
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.snapshot(), before)
        receipt.write_bytes(original)
        private = active / 'github_ed25519'
        private.write_text('invalid fixture')
        with self.assertRaises(controller.Problem) as caught:
            self.execute('setup_status')
        self.assertEqual(caught.exception.code, 'conflict')
        saved = active.with_name('saved')
        active.rename(saved)
        active.symlink_to(saved, target_is_directory=True)
        before = self.snapshot()
        with self.assertRaises(OSError):
            self.execute('setup_status')
        self.assertEqual(self.snapshot(), before)

    def test_setup_status_rejects_private_seed_that_does_not_match_public_key(self):
        key = self.execute('key')
        self.execute('mark', expected=key['public_key'])
        self.execute('identity', name='alice', email='alice@example.invalid')
        private = self.home / '.factory/ssh/active/github_ed25519'
        lines = private.read_bytes().strip().splitlines()
        data = bytearray(base64.b64decode(b''.join(lines[1:-1])))
        offset = len(b'openssh-key-v1\x00')
        for _ in range(3):
            _, offset = controller.ssh_string(data, offset)
        _, offset = controller.ssh_string(data, offset + 4)  # public block
        secret_start = offset + 4
        secret, _ = controller.ssh_string(data, offset)
        secret_offset = 8
        for _ in range(2):
            _, secret_offset = controller.ssh_string(secret, secret_offset)
        data[secret_start + secret_offset + 4] ^= 1  # alter only the private seed
        encoded = base64.b64encode(data)
        wrapped = b'\n'.join(encoded[i:i + 70] for i in range(0, len(encoded), 70))
        private.write_bytes(lines[0] + b'\n' + wrapped + b'\n' + lines[-1] + b'\n')
        before = self.snapshot()
        with self.assertRaises(controller.Problem) as caught:
            self.execute('setup_status')
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.snapshot(), before)

    def test_setup_status_rejects_symlink_and_corrupt_git_identity(self):
        key = self.execute('key')
        self.execute('mark', expected=key['public_key'])
        self.execute('identity', name='alice', email='alice@example.invalid')
        config = self.home / '.factory/git-identity'
        config.write_text('not a git config')
        with self.assertRaises(controller.Problem) as caught:
            self.execute('setup_status')
        self.assertEqual(caught.exception.code, 'conflict')
        config.unlink()
        target = self.home / 'foreign-file'
        target.write_text('keep')
        config.symlink_to(target)
        before = self.snapshot()
        with self.assertRaises(OSError):
            self.execute('setup_status')
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(target.read_text(), 'keep')

    def test_bootstrap_creates_once_reuses_key_and_emits_no_output(self):
        class ExecCalled(BaseException):
            pass

        execute = controller.execute
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(controller, 'execute', side_effect=lambda request: execute(request, str(self.home))):
            with patch.object(controller.os, 'execv', side_effect=ExecCalled) as execv:
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    with self.assertRaises(ExecCalled):
                        controller.bootstrap()
                    first = execute({'op': 'key'}, str(self.home))
                    with self.assertRaises(ExecCalled):
                        controller.bootstrap()
                    second = execute({'op': 'key'}, str(self.home))
        self.assertEqual(first, second)
        self.assertFalse(first['registered'])
        self.assertEqual(execv.call_count, 2)
        execv.assert_called_with('/bin/sleep', ['/bin/sleep', 'infinity'])
        self.assertEqual(stdout.getvalue(), '')
        self.assertEqual(stderr.getvalue(), '')

    def test_bootstrap_failure_is_visible_without_leaking_error_details(self):
        stderr = io.StringIO()
        with patch.object(controller, 'execute', side_effect=RuntimeError('synthetic secret')):
            with patch.object(controller.os, 'execv') as execv:
                with contextlib.redirect_stderr(stderr):
                    with self.assertRaises(SystemExit) as caught:
                        controller.bootstrap()
        self.assertEqual(caught.exception.code, 1)
        self.assertEqual(stderr.getvalue(), 'Factory workspace SSH initialization failed\n')
        execv.assert_not_called()

    def test_key_is_private_stable_and_rotates_only_after_registration(self):
        first = self.execute('key')
        self.assertTrue(first['public_key'].startswith('ssh-ed25519 '))
        self.assertFalse(first['registered'])
        self.assertEqual(first, self.execute('key'))
        key_file = self.home / '.factory/ssh/active/github_ed25519'
        self.assertEqual(stat.S_IMODE(key_file.stat().st_mode), 0o600)
        with self.assertRaises(controller.Problem):
            self.execute('rotate', expected=first['public_key'])
        with self.assertRaises(controller.Problem):
            self.execute('mark', expected='wrong')
        marked = self.execute('mark', expected=first['public_key'])
        self.assertTrue(marked['registered'])
        self.assertEqual(marked, self.execute('mark', expected=first['public_key']))
        rotated = self.execute('rotate', expected=first['public_key'])
        self.assertNotEqual(first['public_key'], rotated['public_key'])
        self.assertFalse(rotated['registered'])
        self.assertEqual(rotated, self.execute('key'))
        self.assertEqual(len(list((self.home / '.factory/ssh').glob('candidate-*'))), 1)
        with self.assertRaises(controller.Problem):
            self.execute('rotate', expected=first['public_key'])

    def test_corrupt_registered_marker_is_never_treated_as_absent(self):
        state = self.execute('key')
        self.execute('mark', expected=state['public_key'])
        marker = self.home / '.factory/ssh/active/registered'
        marker.write_text('{"sha256":"wrong"}')
        with self.assertRaises(controller.Problem) as caught:
            self.execute('key')
        self.assertEqual(caught.exception.code, 'conflict')

    def test_rotation_crash_after_exchange_has_complete_unregistered_new_key(self):
        first = self.execute('key')
        self.execute('mark', expected=first['public_key'])
        original = controller.rename_atomic
        def interrupted(source_dir, source, destination_dir, destination, flags=1):
            original(source_dir, source, destination_dir, destination, flags)
            if flags == 2:
                raise OSError('simulated crash after exchange')
        with patch.object(controller, 'rename_atomic', side_effect=interrupted):
            with self.assertRaises(controller.Problem) as caught:
                self.execute('rotate', expected=first['public_key'])
        self.assertEqual(caught.exception.code, 'uncertain')
        current = self.execute('key')
        self.assertFalse(current['registered'])
        self.assertNotEqual(current['public_key'], first['public_key'])

    def test_atomic_noreplace_keeps_existing_destination(self):
        src = self.home / 'source'
        dest = self.home / 'destination'
        src.mkdir()
        dest.mkdir()
        (src / 'source-witness').write_text('keep source')
        (dest / 'destination-witness').write_text('keep destination')
        parent = os.open(str(self.home), os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(FileExistsError):
                controller.rename_atomic(parent, 'source', parent, 'destination')
        finally:
            os.close(parent)
        self.assertEqual((src / 'source-witness').read_text(), 'keep source')
        self.assertEqual((dest / 'destination-witness').read_text(), 'keep destination')

    def test_user_git_identity_is_not_overwritten(self):
        self.assertEqual(self.execute('identity', name='alice', email='alice@example.invalid'), 'configured')
        config = self.home / '.factory/git-identity'
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(self.home), 'GIT_CONFIG_SYSTEM': str(config)}
        read = lambda key: subprocess.check_output(['git', 'config', '--get', key], env=env, text=True).strip()
        self.assertEqual(read('user.name'), 'alice')
        self.assertEqual(read('user.email'), 'alice@example.invalid')
        (self.home / '.gitconfig').write_text('[user]\n\tname = Preferred Name\n\temail = preferred@example.invalid\n')
        self.execute('identity', name='new-name', email='new@example.invalid')
        self.assertEqual(read('user.name'), 'Preferred Name')
        self.assertEqual(read('user.email'), 'preferred@example.invalid')

    def test_owned_clone_and_crash_receipt(self):
        origin = self.origin()
        self.assertEqual(self.inspect(), 'missing')
        self.assertEqual(self.execute('destination_status', directory='repo'), 'available')
        self.assertEqual(self.clone(origin), 'ready')
        self.assertEqual(self.execute('destination_status', directory='repo'), 'exists')
        target = self.home / 'Documents' / 'repo'
        self.assertEqual((target / 'file.txt').read_text(), 'sample\n')
        self.assertEqual(self.inspect(), 'ready')
        marker = json.loads((target / '.git/factory-import').read_text())
        self.assertEqual(marker, {'project': self.project, 'operation': self.operation})
        with self.assertRaises(controller.Problem) as caught:
            self.clone(origin)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual((target / 'file.txt').read_text(), 'sample\n')

    def test_existing_destination_and_symlinks_are_never_overwritten_or_followed(self):
        origin = self.origin()
        target = self.home / 'Documents' / 'repo'
        target.mkdir()
        (target / 'witness').write_text('keep')
        with self.assertRaises(controller.Problem) as caught:
            self.clone(origin)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual((target / 'witness').read_text(), 'keep')
        target.rename(target.with_name('saved'))
        target.symlink_to(self.home / 'origin', target_is_directory=True)
        self.assertEqual(self.execute('destination_status', directory='repo'), 'exists')
        with self.assertRaises(OSError):
            self.inspect()
        self.assertEqual((self.home / 'origin/file.txt').read_text(), 'sample\n')

    def test_destination_rejects_paths_and_invalid_names(self):
        for name in ('', '.', '..', '../repo', 'repo/nested', 'repo\n'):
            with self.assertRaises(controller.Problem):
                self.execute('destination_status', directory=name)

    def test_noreplace_and_partial_staging_are_recoverable_without_cleanup(self):
        origin = self.origin()
        with patch.object(controller, 'rename_atomic', side_effect=OSError('interrupted')):
            with self.assertRaises(controller.Problem) as caught:
                self.clone(origin)
        self.assertEqual(caught.exception.code, 'uncertain')
        self.assertEqual(self.inspect(), 'staged')
        stage = self.home / '.factory/imports' / self.operation
        self.assertTrue((stage / 'checkout/file.txt').exists())
        # Same operation cannot be adopted or launched twice.
        with self.assertRaises(FileExistsError):
            self.clone(origin)
        self.assertTrue(stage.exists())

    def test_delete_removes_owned_checkout_and_never_a_recreated_namesake(self):
        origin = self.origin()
        self.clone(origin)
        target = self.home / 'Documents' / 'repo'
        request = dict(project=self.project, operation=self.operation, directory='repo')
        self.assertEqual(self.execute('delete', **request), 'deleted')
        self.assertFalse(target.exists())
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')
        target.mkdir()
        (target / 'witness').write_text('keep')
        self.assertEqual(self.execute('delete', **request), 'deleted')
        self.assertEqual((target / 'witness').read_text(), 'keep')

    def test_delete_never_adopts_unmarked_or_symlinked_destination(self):
        origin = self.origin()
        target = self.home / 'Documents' / 'repo'
        target.mkdir()
        (target / 'witness').write_text('keep')
        request = dict(project=self.project, operation=self.operation, directory='repo')
        with self.assertRaises((controller.Problem, OSError)):
            self.execute('delete', **request)
        self.assertEqual((target / 'witness').read_text(), 'keep')
        target.rename(target.with_name('saved'))
        target.symlink_to(origin, target_is_directory=True)
        with self.assertRaises((controller.Problem, OSError)):
            self.execute('delete', **request)
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')

    def test_delete_resumes_after_partial_recursive_cleanup(self):
        origin = self.origin()
        self.clone(origin)
        request = dict(project=self.project, operation=self.operation, directory='repo')
        remove = controller.shutil.rmtree
        def fail_once(path, **kwargs):
            if path == '.git':
                raise OSError('interrupted')
            return remove(path, **kwargs)
        with patch.object(controller.shutil, 'rmtree', side_effect=fail_once):
            with self.assertRaises(OSError):
                self.execute('delete', **request)
        target = self.home / 'Documents' / 'repo'
        target.mkdir()
        (target / 'witness').write_text('keep')
        self.assertEqual(self.execute('delete', **request), 'deleted')
        self.assertEqual((target / 'witness').read_text(), 'keep')

    def test_delete_fails_closed_when_quarantine_is_replaced(self):
        origin = self.origin()
        self.clone(origin)
        request = dict(project=self.project, operation=self.operation, directory='repo')
        with patch.object(controller.shutil, 'rmtree', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError):
                self.execute('delete', **request)
        record = self.home / '.factory/deletions' / self.project
        (record / 'checkout').rename(record / 'saved')
        (record / 'checkout').mkdir()
        (record / 'checkout/witness').write_text('keep')
        with self.assertRaises(controller.Problem) as caught:
            self.execute('delete', **request)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual((record / 'checkout/witness').read_text(), 'keep')

    def test_delete_resumes_owned_feature_cleanup_and_preserves_recreated_group(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        self.ready_feature_snapshot(origin, feature)
        groups = self.home / '.factory/features'
        sibling = groups / str(uuid.uuid4())
        sibling.mkdir()
        (sibling / 'keep').write_text('another project')
        request = dict(project=self.project, operation=self.operation, directory='repo')
        remove = controller.shutil.rmtree

        def fail_feature_removal(path, **kwargs):
            if path == feature:
                raise OSError('interrupted during feature cleanup')
            return remove(path, **kwargs)

        with patch.object(controller.shutil, 'rmtree', side_effect=fail_feature_removal):
            with self.assertRaises(OSError):
                self.execute('delete', **request)

        quarantined = self.home / '.factory/deletions' / self.project / 'features'
        self.assertTrue((quarantined / feature / 'integration.git').is_dir())
        original = groups / self.project
        self.assertFalse(original.exists())
        original.mkdir()
        (original / 'keep').write_text('new namesake')
        self.assertEqual(self.execute('delete', **request), 'deleted')
        self.assertFalse(quarantined.exists())
        self.assertEqual((original / 'keep').read_text(), 'new namesake')
        self.assertEqual((sibling / 'keep').read_text(), 'another project')

    def test_delete_does_not_adopt_an_unmarked_feature_workspace(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        self.feature(origin, feature, 'initialize')
        group = self.home / '.factory/features' / self.project
        unowned = group / str(uuid.uuid4())
        unowned.mkdir(mode=0o700)
        (unowned / 'keep').write_text('unowned')
        with self.assertRaises(controller.Problem) as caught:
            self.execute('delete', project=self.project, operation=self.operation, directory='repo')
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual((unowned / 'keep').read_text(), 'unowned')
        self.assertTrue((group / feature / 'integration.git').is_dir())
        self.assertEqual((self.home / 'Documents/repo/file.txt').read_text(), 'sample\n')

    def test_stopped_import_cleanup_removes_only_owned_checkout_and_is_retryable(self):
        origin = self.origin()
        with patch.object(controller, 'rename_atomic', side_effect=OSError('interrupted')):
            with self.assertRaises(controller.Problem):
                self.clone(origin)
        stage = self.home / '.factory/imports' / self.operation
        (stage / 'keep').write_text('not part of checkout')
        self.assertEqual(self.execute('clean_stage', project=self.project, operation=self.operation), 'cleaned')
        self.assertFalse((stage / 'checkout').exists())
        self.assertTrue((stage / 'factory-operation').exists())
        self.assertEqual((stage / 'keep').read_text(), 'not part of checkout')
        self.assertEqual(self.execute('clean_stage', project=self.project, operation=self.operation), 'cleaned')
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')

    def test_stopped_import_cleanup_can_resume_after_removal_failure(self):
        origin = self.origin()
        with patch.object(controller, 'rename_atomic', side_effect=OSError('interrupted')):
            with self.assertRaises(controller.Problem):
                self.clone(origin)
        stage = self.home / '.factory/imports' / self.operation
        with patch.object(controller.shutil, 'rmtree', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError):
                self.execute('clean_stage', project=self.project, operation=self.operation)
        self.assertTrue((stage / 'factory-operation').exists())
        self.assertEqual(self.execute('clean_stage', project=self.project, operation=self.operation), 'cleaned')
        self.assertFalse((stage / 'checkout').exists())
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')

    def test_stopped_import_cleanup_rejects_foreign_marker_and_symlink(self):
        origin = self.origin()
        with patch.object(controller, 'rename_atomic', side_effect=OSError('interrupted')):
            with self.assertRaises(controller.Problem):
                self.clone(origin)
        stage = self.home / '.factory/imports' / self.operation
        with self.assertRaises(controller.Problem):
            self.execute('clean_stage', project=str(uuid.uuid4()), operation=self.operation)
        self.assertTrue((stage / 'checkout/file.txt').exists())
        (stage / 'checkout').rename(stage / 'saved')
        (stage / 'checkout').symlink_to(origin, target_is_directory=True)
        with self.assertRaises(OSError):
            self.execute('clean_stage', project=self.project, operation=self.operation)
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')

    def test_destination_created_during_clone_is_not_replaced(self):
        origin = self.origin()
        rename = controller.rename_atomic
        target = self.home / 'Documents' / 'repo'
        def collide(source_dir, source, destination_dir, destination, flags=1):
            target.mkdir()
            (target / 'witness').write_text('keep')
            return rename(source_dir, source, destination_dir, destination, flags)
        with patch.object(controller, 'rename_atomic', side_effect=collide):
            with self.assertRaises(controller.Problem) as caught:
                self.clone(origin)
        self.assertEqual(caught.exception.code, 'uncertain')
        self.assertEqual((target / 'witness').read_text(), 'keep')
        self.assertTrue((self.home / '.factory/imports' / self.operation / 'checkout/file.txt').exists())

    def test_checkout_swapped_for_symlink_never_reports_ready(self):
        origin = self.origin()
        rename = controller.rename_atomic
        def swap(source_dir, source, destination_dir, destination, flags=1):
            if source == 'checkout':
                os.rename('checkout', 'saved-checkout', src_dir_fd=source_dir, dst_dir_fd=source_dir)
                os.symlink(str(origin), 'checkout', dir_fd=source_dir)
            return rename(source_dir, source, destination_dir, destination, flags)
        with patch.object(controller, 'rename_atomic', side_effect=swap):
            with self.assertRaises(controller.Problem) as caught:
                self.clone(origin)
        self.assertEqual(caught.exception.code, 'uncertain')
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')
        self.assertTrue((self.home / '.factory/imports' / self.operation / 'saved-checkout').is_dir())

    def test_ssh_configuration_and_proxy_acl_are_narrow(self):
        config = Path(__file__).with_name('github-ssh-config')
        known_hosts = Path(__file__).with_name('github-known-hosts')
        result = subprocess.run(['/usr/bin/ssh', '-G', '-F', str(config), 'git@github.com'],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, check=True).stdout
        self.assertIn('hostname ssh.github.com\n', result)
        self.assertIn('port 443\n', result)
        self.assertIn('stricthostkeychecking true\n', result)
        self.assertIn('batchmode yes\n', result)
        self.assertIn('proxycommand /usr/bin/nc -w 15 -X connect -x factory-egress:3128 %h %p\n', result)
        host = subprocess.run(['/usr/bin/ssh-keygen', '-F', '[ssh.github.com]:443', '-f', str(known_hosts)],
                              stdout=subprocess.PIPE, text=True, check=True).stdout
        self.assertIn('ssh-ed25519', host)
        fingerprint = subprocess.run(['/usr/bin/ssh-keygen', '-l', '-f', str(known_hosts)],
                                     stdout=subprocess.PIPE, text=True, check=True).stdout
        self.assertIn('SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU', fingerprint)
        acl = (Path(__file__).parent.parent / 'egress/squid.conf').read_text()
        line = next(line for line in acl.splitlines() if line.startswith('acl providers dstdomain -n '))
        domains = line.split()[4:]
        self.assertIn('ssh.github.com', domains)
        self.assertNotIn('github.com', domains)
        self.assertIn('http_access deny !CONNECT', acl)
        self.assertIn('http_access deny !tls_port', acl)
        self.assertIn('http_access deny forbidden', acl)
        self.assertIn('http_access deny host_addresses', acl)
        self.assertIn('http_access deny all', acl)

    def test_feature_workspaces_capture_and_integrate_only_approved_changes(self):
        origin = self.origin()
        self.clone(origin)
        member_file = self.home / 'Documents/repo/file.txt'
        member_file.write_text('uncommitted member change\n')
        feature = str(uuid.uuid4())
        branch = self.feature(origin, feature, 'initialize')
        base = branch['base_oid']
        self.assertEqual(self.feature(origin, feature, 'initialize'), branch)

        def prepare():
            attempt = str(uuid.uuid4())
            fields = {'attempt': attempt, 'base_oid': base, 'writable': True}
            self.feature(origin, feature, 'prepare', **fields)
            return self.feature_checkout(feature, attempt), fields

        first, first_request = prepare()
        second, second_request = prepare()
        (first / 'file.txt').write_text('approved edit\n')
        (first / 'outside.txt').write_text('out of scope\n')
        (second / 'other.txt').write_text('parallel edit\n')
        self.assertEqual((second / 'file.txt').read_text(), 'sample\n')
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'checkpoint', scope=['file.txt'], **first_request)
        self.assertEqual(caught.exception.code, 'scope_violation')
        (first / 'outside.txt').unlink()
        subprocess.run(['git', '-C', str(first), 'config', 'core.hooksPath', '/invalid'], check=True)
        monitor_ran = self.home / 'worker-monitor-ran'
        monitor = self.home / 'worker-monitor'
        monitor.write_text(f"#!/bin/sh\n/usr/bin/touch '{monitor_ran}'\n")
        monitor.chmod(0o700)
        subprocess.run(['git', '-C', str(first), 'config', 'core.fsmonitor', str(monitor)], check=True)

        first_checkpoint = self.feature(origin, feature, 'checkpoint', scope=['file.txt'], **first_request)
        self.assertFalse(monitor_ran.exists())
        self.assertEqual(
            self.feature(origin, feature, 'checkpoint', scope=['file.txt'], **first_request), first_checkpoint,
        )
        self.feature(origin, feature, 'checkpoint', scope=['other.txt'], **second_request)
        self.assertEqual(self.feature(origin, feature, 'integrate', expected_head=base,
                                      scope=['file.txt'], **first_request), first_checkpoint)
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'integrate', expected_head=base, scope=['other.txt'], **second_request)
        self.assertEqual(caught.exception.code, 'conflict')
        head = self.feature(origin, feature, 'integrate', expected_head=first_checkpoint,
                            scope=['other.txt'], **second_request)
        snapshot = self.feature(origin, feature, 'snapshot')
        self.assertEqual(snapshot['git']['head_oid'], head)
        self.assertEqual(snapshot['git']['base_oid'], base)
        self.assertEqual(snapshot['changed_paths'], ['file.txt', 'other.txt'])
        self.assertIn('+approved edit', snapshot['patch'])
        self.assertIn('+parallel edit', snapshot['patch'])
        self.assertEqual(snapshot['git']['patch_digest'], controller.hashlib.sha256(snapshot['patch'].encode()).hexdigest())
        self.assertEqual(member_file.read_text(), 'uncommitted member change\n')
        self.assertEqual((origin / 'file.txt').read_text(), 'sample\n')
        self.assertEqual(self.feature(origin, feature, 'remote'),
                         {'base_oid': base, 'head_oid': None, 'managed_head_oid': None})

        review = str(uuid.uuid4())
        self.feature(origin, feature, 'prepare', attempt=review, base_oid=head, writable=False)
        self.assertEqual((self.feature_checkout(feature, review) / 'other.txt').read_text(), 'parallel edit\n')
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'checkpoint', attempt=review, base_oid=head,
                         writable=True, scope=['other.txt'])
        self.assertEqual(caught.exception.code, 'conflict')

        retry = str(uuid.uuid4())
        self.feature(origin, feature, 'prepare', attempt=retry, base_oid=first_checkpoint, writable=True)
        self.assertEqual((self.feature_checkout(feature, retry) / 'file.txt').read_text(), 'approved edit\n')

        foreign = str(uuid.uuid4())
        unowned = self.feature_checkout(feature, foreign)
        unowned.mkdir(mode=0o700, parents=True)
        (unowned / 'keep').write_text('not owned by this operation')
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'prepare', attempt=foreign, base_oid=head, writable=True)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual((unowned / 'keep').read_text(), 'not owned by this operation')

    def test_feature_initialization_pins_the_fetched_base_when_remote_advances(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        get_remote_default = controller.remote_default
        remote_git = ['git', '-C', str(origin)]

        def advance_after_advertisement(remote):
            advertisement = get_remote_default(remote)
            (origin / 'upstream.txt').write_text('updated upstream\n')
            subprocess.run([*remote_git, 'add', 'upstream.txt'], check=True)
            subprocess.run([*remote_git, 'commit', '-qm', 'advance during initialization'], check=True)
            return advertisement

        with patch.object(controller, 'remote_default', side_effect=advance_after_advertisement):
            branch = self.feature(origin, feature, 'initialize')
        new_base = subprocess.check_output([*remote_git, 'rev-parse', 'HEAD'], text=True).strip()
        self.assertEqual(branch['base_oid'], new_base)
        self.assertEqual(self.feature(origin, feature, 'initialize'), branch)
        result = self.feature(origin, feature, 'snapshot')
        self.assertEqual(result['git']['head_oid'], new_base)
        self.assertEqual(result['changed_paths'], [])
        self.assertFalse((self.home / 'Documents/repo/upstream.txt').exists())

    def test_rejected_feature_candidate_never_becomes_usable_after_the_original_origin_returns(self):
        origin = self.origin()
        self.clone(origin)
        original_head = subprocess.check_output(['git', '-C', str(origin), 'rev-parse', 'HEAD'], text=True).strip()
        saved_origin = origin.with_name('original')
        origin.rename(saved_origin)
        replacement = self.origin()
        (replacement / 'file.txt').write_text('replacement repository\n')
        subprocess.run(['git', '-C', str(replacement), 'commit', '-qam', 'replacement'], check=True)

        feature = str(uuid.uuid4())
        rejected_id = str(uuid.uuid4())
        rejected = self.feature(replacement, feature, 'stage_initialization', candidate=rejected_id)
        self.assertNotEqual(rejected['base_oid'], original_head)
        with self.assertRaises(controller.Problem):
            self.feature(replacement, feature, 'snapshot')
        with self.assertRaises(controller.Problem):
            self.feature(replacement, feature, 'prepare', attempt=str(uuid.uuid4()),
                         base_oid=rejected['base_oid'], writable=False)

        replacement.rename(replacement.with_name('rejected-origin'))
        saved_origin.rename(origin)
        candidate = str(uuid.uuid4())
        branch = self.feature(origin, feature, 'stage_initialization', candidate=candidate)
        self.assertEqual(branch['base_oid'], original_head)
        with self.assertRaises(controller.Problem):
            self.feature(origin, feature, 'confirm_initialization', candidate=rejected_id, expected_branch=rejected)
        self.feature(origin, feature, 'confirm_initialization', candidate=candidate, expected_branch=branch)
        attempt = str(uuid.uuid4())
        self.feature(origin, feature, 'prepare', attempt=attempt, base_oid=original_head, writable=False)
        self.assertEqual((self.feature_checkout(feature, attempt) / 'file.txt').read_text(), 'sample\n')

    def test_initialization_and_deletion_are_fenced_in_both_orders(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        entered, proceed = threading.Event(), threading.Event()
        inspect = controller.inspect_project
        request = dict(project=self.project, operation=self.operation, directory='repo')
        group = self.home / '.factory/features' / self.project

        def pause_initialization(*args):
            result = inspect(*args)
            entered.set()
            self.assertTrue(proceed.wait(5))
            return result

        with patch.object(controller, 'inspect_project', side_effect=pause_initialization):
            with ThreadPoolExecutor(max_workers=1) as executor:
                stage = executor.submit(self.feature, origin, feature, 'stage_initialization',
                                        candidate=str(uuid.uuid4()))
                try:
                    self.assertTrue(entered.wait(5))
                    with self.assertRaises(BlockingIOError):
                        self.execute('delete', **request)
                finally:
                    proceed.set()
                stage.result(timeout=5)
        self.assertTrue(group.is_dir())

        entered.clear()
        proceed.clear()
        verify = controller.verify_feature_group

        def pause_deletion(*args):
            result = verify(*args)
            entered.set()
            self.assertTrue(proceed.wait(5))
            return result

        with patch.object(controller, 'verify_feature_group', side_effect=pause_deletion):
            with ThreadPoolExecutor(max_workers=1) as executor:
                deletion = executor.submit(self.execute, 'delete', **request)
                try:
                    self.assertTrue(entered.wait(5))
                    with self.assertRaises(BlockingIOError):
                        self.feature(origin, feature, 'stage_initialization', candidate=str(uuid.uuid4()))
                finally:
                    proceed.set()
                self.assertEqual(deletion.result(timeout=5), 'deleted')
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'stage_initialization', candidate=str(uuid.uuid4()))
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertFalse(group.exists())

    def test_delete_receipt_blocks_new_candidates_after_interrupted_quarantine(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        self.feature(origin, feature, 'initialize')
        rename = controller.rename_atomic

        def interrupt_checkout(source_dir, source, destination_dir, destination, flags=1):
            if destination == 'checkout':
                raise OSError('interrupted before removing the project marker')
            return rename(source_dir, source, destination_dir, destination, flags)

        request = dict(project=self.project, operation=self.operation, directory='repo')
        with patch.object(controller, 'rename_atomic', side_effect=interrupt_checkout):
            with self.assertRaises(OSError):
                self.execute('delete', **request)
        self.assertEqual(self.inspect(), 'ready')
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, str(uuid.uuid4()), 'stage_initialization', candidate=str(uuid.uuid4()))
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.execute('delete', **request), 'deleted')

    def test_integration_keeps_previous_result_when_cumulative_diff_cannot_be_displayed(self):
        origin = self.origin()
        self.clone(origin)
        for limit, size, first_path, second_path, content in (
            ('FEATURE_PATCH_LIMIT', 1000, 'first.txt', 'second.txt', 'x' * 650 + '\n'),
            ('FEATURE_PATH_LIMIT', 160, 'first-' + 'x' * 85, 'second-' + 'x' * 85, 'short\n'),
        ):
            with self.subTest(limit=limit):
                # Each edit fits on its own, but the integrated result would
                # block every subsequent snapshot read.
                feature = str(uuid.uuid4())
                base = self.feature(origin, feature, 'initialize')['base_oid']
                current = base
                with patch.object(controller, limit, size):
                    for path in (first_path, second_path):
                        fields = {'attempt': str(uuid.uuid4()), 'base_oid': base, 'writable': True}
                        self.feature(origin, feature, 'prepare', **fields)
                        (self.feature_checkout(feature, fields['attempt']) / path).write_text(content)
                        self.feature(origin, feature, 'checkpoint', scope=[path], **fields)
                        if path == first_path:
                            current = self.feature(origin, feature, 'integrate', expected_head=current,
                                                   scope=[path], **fields)
                        else:
                            with self.assertRaises(controller.Problem) as caught:
                                self.feature(origin, feature, 'integrate', expected_head=current,
                                             scope=[path], **fields)
                            self.assertEqual(caught.exception.code, 'diff_unrenderable')
                    snapshot = self.feature(origin, feature, 'snapshot')
                    self.assertEqual(snapshot['git']['head_oid'], current)
                    self.assertEqual(snapshot['changed_paths'], [first_path])

    def ready_feature_snapshot(self, origin, feature):
        branch = self.feature(origin, feature, 'initialize')
        base = branch['base_oid']
        fields = {'attempt': str(uuid.uuid4()), 'base_oid': base, 'writable': True}
        self.feature(origin, feature, 'prepare', **fields)
        checkout = self.feature_checkout(feature, fields['attempt'])
        (checkout / 'file.txt').write_text('ready for review\n')
        self.feature(origin, feature, 'checkpoint', scope=['file.txt'], **fields)
        self.feature(origin, feature, 'integrate', expected_head=base, scope=['file.txt'], **fields)
        return branch, self.feature(origin, feature, 'snapshot')

    def test_feature_push_refuses_a_competing_branch_and_reconciles_an_exact_push(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        branch, result = self.ready_feature_snapshot(origin, feature)
        base, ref = branch['base_oid'], branch['head_ref']
        snapshot = result['git']
        head = snapshot['head_oid']
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'push', **{**snapshot, 'head_oid': base})
        self.assertEqual(caught.exception.code, 'conflict')

        remote_git = ['git', '-C', str(origin)]
        subprocess.run([*remote_git, 'update-ref', ref, base], check=True)
        inspect = controller.current_remote
        # Simulate another publisher creating the ref between the preflight
        # and Git push. The ordinary push hook must also reject a fast-forward.
        with patch.object(controller, 'current_remote', side_effect=[
            {'base_oid': base, 'head_oid': None}, inspect(str(origin), branch),
        ]):
            with self.assertRaises(controller.Problem) as caught:
                self.feature(origin, feature, 'push', **snapshot)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(inspect(str(origin), branch)['head_oid'], base)

        subprocess.run([*remote_git, 'update-ref', '-d', ref, base], check=True)
        self.assertEqual(self.feature(origin, feature, 'push', **snapshot), 'pushed')
        self.assertEqual(inspect(str(origin), branch)['head_oid'], head)
        self.assertEqual(self.feature(origin, feature, 'push', **snapshot), 'pushed')

    def test_feature_recovers_its_proven_remote_head_when_base_moves_after_push(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        branch, result = self.ready_feature_snapshot(origin, feature)
        approved = result['git']
        remote_git = ['git', '-C', str(origin)]
        inspect = controller.current_remote
        queries = 0

        def advance_after_push(remote, refs):
            nonlocal queries
            queries += 1
            if queries == 2:
                (origin / 'upstream.txt').write_text('base advanced during publication\n')
                subprocess.run([*remote_git, 'add', 'upstream.txt'], check=True)
                subprocess.run([*remote_git, 'commit', '-qm', 'base advance after push'], check=True)
            return inspect(remote, refs)

        with patch.object(controller, 'current_remote', side_effect=advance_after_push):
            with self.assertRaises(controller.Problem) as caught:
                self.feature(origin, feature, 'push', **approved)
        self.assertEqual(caught.exception.code, 'conflict')
        observed = self.feature(origin, feature, 'remote')
        self.assertEqual(observed['head_oid'], approved['head_oid'])
        self.assertEqual(observed['managed_head_oid'], approved['head_oid'])
        self.assertNotEqual(observed['base_oid'], approved['base_oid'])

        updated = self.feature(origin, feature, 'refresh', **approved)['git']
        self.assertEqual(updated['base_oid'], observed['base_oid'])
        self.assertNotEqual(updated['head_oid'], approved['head_oid'])
        with self.assertRaises(controller.Problem):
            self.feature(origin, feature, 'push', **approved)
        self.assertEqual(self.feature(origin, feature, 'push', **updated), 'pushed')
        self.assertEqual(self.feature(origin, feature, 'remote'), {
            'base_oid': updated['base_oid'], 'head_oid': updated['head_oid'],
            'managed_head_oid': updated['head_oid'],
        })
        subprocess.run([*remote_git, 'update-ref', branch['head_ref'], approved['head_oid']], check=True)
        self.assertIsNone(self.feature(origin, feature, 'remote')['managed_head_oid'])
        with self.assertRaises(controller.Problem):
            self.feature(origin, feature, 'push', **updated)
        self.assertEqual(inspect(str(origin), branch)['head_oid'], approved['head_oid'])

    def test_feature_refreshes_only_owned_refs_and_requires_review_of_the_new_snapshot(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        branch, result = self.ready_feature_snapshot(origin, feature)
        remote_git = ['git', '-C', str(origin)]
        (origin / 'upstream.txt').write_text('upstream work\n')
        subprocess.run([*remote_git, 'add', 'upstream.txt'], check=True)
        subprocess.run([*remote_git, 'commit', '-qm', 'upstream advance'], check=True)
        new_base = subprocess.check_output([*remote_git, 'rev-parse', 'HEAD'], text=True).strip()

        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'push', **result['git'])
        self.assertEqual(caught.exception.code, 'conflict')
        refreshed = self.feature(origin, feature, 'refresh', **result['git'])
        self.assertEqual(refreshed, self.feature(origin, feature, 'snapshot'))
        self.assertEqual(refreshed['git']['base_oid'], new_base)
        self.assertNotEqual(refreshed['git']['head_oid'], result['git']['head_oid'])
        self.assertEqual(refreshed['changed_paths'], ['file.txt'])
        self.assertIn('+ready for review', refreshed['patch'])
        self.assertFalse((self.home / 'Documents/repo/upstream.txt').exists())
        self.assertEqual(self.feature(origin, feature, 'initialize'), {**branch, 'base_oid': new_base})
        self.assertEqual(self.feature(origin, feature, 'remote'),
                         {'base_oid': new_base, 'head_oid': None, 'managed_head_oid': None})
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'push', **result['git'])
        self.assertEqual(caught.exception.code, 'conflict')

    def test_feature_base_refresh_keeps_the_prior_result_after_a_merge_conflict(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        _, result = self.ready_feature_snapshot(origin, feature)
        (origin / 'file.txt').write_text('conflicting upstream edit\n')
        subprocess.run(['git', '-C', str(origin), 'commit', '-qam', 'conflicting advance'], check=True)
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'refresh', **result['git'])
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.feature(origin, feature, 'snapshot'), result)

    def test_refresh_rejects_a_replacement_repository_with_shared_ancestry(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        _, result = self.ready_feature_snapshot(origin, feature)
        replacement = origin.with_name('replacement')
        subprocess.run(['git', 'clone', '-q', str(origin), str(replacement)], check=True)
        replacement_git = ['git', '-C', str(replacement)]
        (replacement / 'private.txt').write_text('replacement repository\n')
        subprocess.run([*replacement_git, 'add', 'private.txt'], check=True)
        subprocess.run([*replacement_git, '-c', 'user.name=Fixture', '-c',
                        'user.email=fixture@example.invalid', 'commit', '-qm', 'fork change'], check=True)

        saved = origin.with_name('canonical')
        origin.rename(saved)
        replacement.rename(origin)
        candidate = str(uuid.uuid4())
        proposed = self.feature(origin, feature, 'stage_refresh', **result['git'], candidate=candidate)
        self.assertNotEqual(proposed['base_oid'], result['git']['base_oid'])
        self.assertEqual(self.feature(origin, feature, 'snapshot'), result)
        rejected_checkout = str(uuid.uuid4())
        with self.assertRaises(controller.Problem):
            self.feature(origin, feature, 'prepare', attempt=rejected_checkout, writable=False,
                         base_oid=proposed['head_oid'])
        self.assertFalse(self.feature_checkout(feature, rejected_checkout).exists())

        origin.rename(replacement)
        saved.rename(origin)
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'confirm_refresh', **result['git'], candidate=candidate,
                         proposed_snapshot=proposed)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.feature(origin, feature, 'snapshot'), result)

    def test_refresh_confirmation_rejects_changed_managed_head_and_stale_proof(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        branch, result = self.ready_feature_snapshot(origin, feature)
        snapshot = result['git']
        self.feature(origin, feature, 'push', **snapshot)
        remote_git = ['git', '-C', str(origin)]
        (origin / 'upstream.txt').write_text('new base\n')
        subprocess.run([*remote_git, 'add', 'upstream.txt'], check=True)
        subprocess.run([*remote_git, 'commit', '-qm', 'advance'], check=True)
        candidate = str(uuid.uuid4())
        proposed = self.feature(origin, feature, 'stage_refresh', **snapshot, candidate=candidate)
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'confirm_refresh', **snapshot, candidate=candidate,
                         proposed_snapshot={**proposed, 'patch_digest': '0' * 64})
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.feature(origin, feature, 'snapshot'), result)

        subprocess.run([*remote_git, 'update-ref', branch['head_ref'], snapshot['base_oid']], check=True)
        with self.assertRaises(controller.Problem) as caught:
            self.feature(origin, feature, 'confirm_refresh', **snapshot, candidate=candidate,
                         proposed_snapshot=proposed)
        self.assertEqual(caught.exception.code, 'conflict')
        self.assertEqual(self.feature(origin, feature, 'snapshot'), result)

    def test_unsafe_home_and_forged_stage_are_rejected(self):
        foreign = self.home / 'foreign'
        foreign.mkdir()
        (self.home / '.factory').symlink_to(foreign, target_is_directory=True)
        with self.assertRaises(OSError):
            self.execute('key')
        self.assertEqual(list(foreign.iterdir()), [])
        (self.home / '.factory').unlink()
        stage = self.home / '.factory/imports' / self.operation
        stage.mkdir(mode=0o700, parents=True)
        (stage / 'factory-operation').write_text(json.dumps({'project': str(uuid.uuid4()), 'operation': self.operation}))
        with self.assertRaises(controller.Problem) as caught:
            self.inspect()
        self.assertEqual(caught.exception.code, 'conflict')


if __name__ == '__main__':
    unittest.main()
