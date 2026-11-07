#!/usr/bin/python3
"""Local synthetic filesystem and Git regressions; no real accounts or network."""
import importlib.util
import base64
import contextlib
import fcntl
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
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
            return self.execute('feature_' + op, project=self.project, operation=self.operation,
                                directory='repo', repository='owner/repo', feature=feature, **fields)

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
        self.assertEqual(self.feature(origin, feature, 'remote'), {'base_oid': base, 'head_oid': None})

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

    def test_feature_push_refuses_a_competing_branch_and_reconciles_an_exact_push(self):
        origin = self.origin()
        self.clone(origin)
        feature = str(uuid.uuid4())
        branch = self.feature(origin, feature, 'initialize')
        base, ref = branch['base_oid'], branch['head_ref']
        fields = {'attempt': str(uuid.uuid4()), 'base_oid': base, 'writable': True}
        self.feature(origin, feature, 'prepare', **fields)
        checkout = self.feature_checkout(feature, fields['attempt'])
        (checkout / 'file.txt').write_text('ready for review\n')
        self.feature(origin, feature, 'checkpoint', scope=['file.txt'], **fields)
        head = self.feature(origin, feature, 'integrate', expected_head=base, scope=['file.txt'], **fields)
        snapshot = self.feature(origin, feature, 'snapshot')['git']
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
