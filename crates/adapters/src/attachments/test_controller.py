#!/usr/bin/python3
"""Local synthetic filesystem regressions for the attachment controller; no network."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
import uuid
from unittest.mock import patch

MODULE_PATH = Path(__file__).with_name('controller.py')
spec = importlib.util.spec_from_file_location('factory_attachment_controller', MODULE_PATH)
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)


def frames(*chunks, terminated=True):
    data = b''.join(len(chunk).to_bytes(4, 'big') + chunk for chunk in chunks)
    return data + (b'\0\0\0\0' if terminated else b'')


class AttachmentControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='factory-attachment-test-')
        self.home = Path(self.tmp.name) / 'home'
        self.home.mkdir(mode=0o700)
        (self.home / '.factory').mkdir(mode=0o700)
        self.work = self.home / 'Documents'
        self.work.mkdir()
        self.conversation = str(uuid.uuid4())
        self.attachment = str(uuid.uuid4())
        self.project = None

    def tearDown(self):
        self.tmp.cleanup()

    def store(self, project=None, conversation=None):
        scope = 'chats' if project is None else project
        return self.home / '.factory/attachments' / scope / (conversation or self.conversation)

    def location(self, **fields):
        return {'project': self.project, 'conversation': self.conversation,
                'attachment': self.attachment, 'name': 'note.txt', **fields}

    def run_main(self, request, payload=b''):
        stdin = io.BytesIO(json.dumps(request).encode() + b'\n' + payload)
        stdout = io.BytesIO()
        with patch.object(controller.sys, 'stdin', SimpleNamespace(buffer=stdin)), \
                patch.object(controller.sys, 'stdout', SimpleNamespace(buffer=stdout)):
            controller.main(str(self.home))
        return stdout.getvalue()

    def call(self, request, payload=b''):
        output = self.run_main(request, payload)
        line, _, rest = output.partition(b'\n')
        response = json.loads(line)
        return response, rest

    def put(self, data=b'hello', **fields):
        response, _ = self.call({'op': 'put', **self.location(**fields)}, frames(data))
        return response

    def error(self, request, payload=b''):
        response, _ = self.call(request, payload)
        self.assertFalse(response['ok'], response)
        return response['error']

    def test_put_stores_private_file_with_digest_and_head(self):
        data = b'<svg xmlns="x"/>' + b'a' * 20000
        response, _ = self.call({'op': 'put', **self.location()}, frames(data[:7], data[7:]))
        result = response['result']
        self.assertEqual(result['size'], len(data))
        self.assertEqual(result['sha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual(bytes.fromhex(result['head']), data[:controller.MAX_HEAD])
        directory = self.store() / self.attachment
        self.assertEqual((directory / 'note.txt').read_bytes(), data)
        self.assertEqual(sorted(path.name for path in directory.iterdir()), ['note.txt'])
        for path in (self.home / '.factory/attachments', self.store(), directory):
            self.assertEqual(path.stat().st_mode & 0o777, 0o700)
        self.assertEqual((directory / 'note.txt').stat().st_mode & 0o777, 0o600)

    def test_project_conversations_live_under_their_project(self):
        self.project = str(uuid.uuid4())
        self.assertTrue(self.put()['ok'])
        self.assertTrue((self.store(self.project) / self.attachment / 'note.txt').is_file())
        self.assertFalse((self.home / '.factory/attachments/chats').exists())

    def test_empty_file_is_stored(self):
        response, _ = self.call({'op': 'put', **self.location()}, frames())
        self.assertEqual(response['result']['size'], 0)
        self.assertEqual(response['result']['head'], '')

    def test_oversized_upload_is_rejected_and_leaves_nothing(self):
        with patch.object(controller, 'MAX_BYTES', 10):
            self.assertEqual(self.error({'op': 'put', **self.location()}, frames(b'x' * 6, b'y' * 5)),
                             'too_large')
            self.assertTrue(self.put(b'x' * 10)['ok'])
        self.assertEqual(sorted(path.name for path in (self.store() / self.attachment).iterdir()), ['note.txt'])
        other = str(uuid.uuid4())
        with patch.object(controller, 'MAX_BYTES', 10):
            self.assertEqual(self.error({'op': 'put', **self.location(attachment=other)}, frames(b'x' * 11)),
                             'too_large')
        self.assertEqual(list((self.store() / other).iterdir()), [])

    def test_abandoned_upload_never_becomes_a_file(self):
        error = self.error({'op': 'put', **self.location()}, frames(b'partial', terminated=False))
        self.assertEqual(error, 'invalid')
        self.assertEqual(list((self.store() / self.attachment).iterdir()), [])
        self.assertEqual(self.error({'op': 'put', **self.location()}, b'\0\0'), 'invalid')

    def test_oversized_frame_is_rejected(self):
        header = (controller.MAX_FRAME + 1).to_bytes(4, 'big')
        self.assertEqual(self.error({'op': 'put', **self.location()}, header), 'invalid')
        self.assertEqual(list((self.store() / self.attachment).iterdir()), [])

    def test_existing_file_is_never_replaced(self):
        self.assertTrue(self.put(b'first')['ok'])
        self.assertEqual(self.error({'op': 'put', **self.location()}, frames(b'second')), 'conflict')
        directory = self.store() / self.attachment
        self.assertEqual((directory / 'note.txt').read_bytes(), b'first')
        self.assertEqual(sorted(path.name for path in directory.iterdir()), ['note.txt'])

    def test_stale_partial_file_is_not_adopted(self):
        directory = self.store() / self.attachment
        directory.mkdir(parents=True, mode=0o700)
        (directory / '.partial').write_bytes(b'stale')
        self.assertEqual(self.error({'op': 'put', **self.location()}, frames(b'new')), 'conflict')
        self.assertEqual((directory / '.partial').read_bytes(), b'stale')

    def test_names_must_be_one_safe_component(self):
        for name in ('', '.', '..', '.partial', 'a/b', '../x', 'a\nb', 'a\0b', 'x' * 256, '\udcff'):
            self.assertEqual(self.error({'op': 'put', **self.location(name=name)}, frames(b'x')), 'invalid', name)
        self.assertFalse((self.home / '.factory/attachments').exists())
        for name in ('.hidden', 'été 1.png', 'x' * 255):
            self.assertTrue(self.put(name=name, attachment=str(uuid.uuid4()))['ok'], name)

    def test_identifiers_must_be_canonical_uuids(self):
        for fields in ({'conversation': '../x'}, {'attachment': '..'}, {'attachment': self.attachment.upper()},
                       {'attachment': str(uuid.UUID(int=0))}, {'project': 'chats'}, {'project': 5}):
            self.assertEqual(self.error({'op': 'put', **self.location(**fields)}, frames(b'x')), 'invalid', fields)

    def test_unknown_operations_and_fields_are_invalid(self):
        self.assertEqual(self.error({'op': 'shell'}), 'invalid')
        self.assertEqual(self.error({'op': 'put', **self.location(), 'extra': 1}, frames(b'x')), 'invalid')
        self.assertEqual(self.error({'op': 'put', 'conversation': self.conversation}), 'invalid')
        self.assertEqual(self.error([]), 'invalid')

    def test_request_line_is_bounded_and_terminated(self):
        stdout = io.BytesIO()
        for data in (b'{"op":"put"}', b'x' * (controller.MAX_REQUEST + 2) + b'\n'):
            with patch.object(controller.sys, 'stdin', SimpleNamespace(buffer=io.BytesIO(data))), \
                    patch.object(controller.sys, 'stdout', SimpleNamespace(buffer=stdout)):
                controller.main(str(self.home))
        self.assertEqual(stdout.getvalue().count(b'"invalid"'), 2)

    def test_group_accessible_store_directories_are_refused(self):
        (self.home / '.factory/attachments').mkdir(mode=0o700)
        (self.home / '.factory/attachments').chmod(0o750)
        self.assertEqual(self.error({'op': 'put', **self.location()}, frames(b'x')), 'conflict')

    def test_symlinked_store_directories_are_not_followed(self):
        outside = Path(self.tmp.name) / 'outside'
        outside.mkdir()
        (self.home / '.factory/attachments').symlink_to(outside)
        self.assertNotEqual(self.error({'op': 'put', **self.location()}, frames(b'x')), None)
        self.assertEqual(list(outside.iterdir()), [])
        self.assertNotEqual(self.error({'op': 'delete_conversation', 'project': None,
                                        'conversation': self.conversation}), None)
        self.assertTrue(outside.exists())

    def read(self, offset, length, **fields):
        response, data = self.call({'op': 'read', **self.location(**fields), 'offset': offset, 'length': length})
        self.assertTrue(response['ok'], response)
        self.assertEqual(response['result']['length'], len(data))
        return data

    def test_read_returns_bounded_ranges(self):
        data = bytes(range(256)) * 600
        self.put(data)
        self.assertEqual(self.read(0, len(data)), data)
        self.assertEqual(self.read(100, 70000), data[100:70100])
        self.assertEqual(self.read(len(data) - 3, 100), data[-3:])
        self.assertEqual(self.read(len(data), 10), b'')
        self.assertEqual(self.read(len(data) + 99, 10), b'')
        self.assertEqual(self.read(5, 0), b'')
        for offset, length in ((-1, 1), (0, controller.MAX_READ + 1), (True, 1), (0, 1.5), (0, '1')):
            self.assertEqual(self.error({'op': 'read', **self.location(), 'offset': offset, 'length': length}),
                             'invalid', (offset, length))

    def test_read_of_missing_or_unfinished_file_is_not_found(self):
        request = {'op': 'read', **self.location(), 'offset': 0, 'length': 1}
        self.assertEqual(self.error(request), 'not_found')
        self.put()
        directory = self.store() / self.attachment
        (directory / 'note.txt').unlink()
        (directory / '.partial').write_bytes(b'x')
        self.assertEqual(self.error(request), 'not_found')
        self.assertEqual(self.error({**request, 'name': '.partial'}), 'invalid')

    def test_read_never_follows_a_symlinked_file(self):
        self.put()
        secret = Path(self.tmp.name) / 'secret'
        secret.write_text('secret')
        link = self.store() / self.attachment / 'link'
        link.symlink_to(secret)
        error = self.error({'op': 'read', **self.location(name='link'), 'offset': 0, 'length': 10})
        self.assertIn(error, ('conflict', 'not_found'))

    def test_read_stream_fails_loudly_when_a_file_shrinks(self):
        self.put(b'x' * 100)
        fd = os.open(self.store() / self.attachment / 'note.txt', os.O_RDONLY)
        stdout = io.BytesIO()
        with patch.object(controller.sys, 'stdout', SimpleNamespace(buffer=stdout)), \
                patch.object(controller.os, 'pread', return_value=b''):
            with self.assertRaises(SystemExit) as raised:
                controller.stream(fd, 0, 100)
        self.assertEqual(raised.exception.code, 3)
        with self.assertRaises(OSError):
            os.fstat(fd)

    def capture(self, reference, roots=None, **fields):
        roots = [str(self.work)] if roots is None else roots
        return self.call({'op': 'capture', **self.location(), 'reference': str(reference), 'roots': roots,
                          **fields})[0]

    def test_capture_copies_a_regular_file_from_an_allowed_root(self):
        data = b'\x89PNG\r\n\x1a\n' + b'z' * 3000
        source = self.work / 'out' / 'chart.png'
        source.parent.mkdir()
        source.write_bytes(data)
        response = self.capture(source, name='chart.png')
        self.assertEqual(response['result']['size'], len(data))
        self.assertEqual(response['result']['sha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual((self.store() / self.attachment / 'chart.png').read_bytes(), data)
        self.assertEqual(source.read_bytes(), data)

    def test_capture_requires_a_path_strictly_inside_a_root(self):
        (self.work / 'a.txt').write_text('a')
        other = self.home / 'Other'
        other.mkdir()
        (other / 'b.txt').write_text('b')
        for reference in (self.work, other / 'b.txt', self.home / '.factory' / 'x'):
            self.assertEqual(self.capture(reference)['error'], 'invalid', reference)
        for reference in (f'{self.work}/../Other/b.txt', f'{self.work}//a.txt', f'{self.work}/./a.txt',
                          'a.txt', f'{self.work}/a.txt\n', f'{self.work}/'):
            self.assertEqual(self.capture(reference)['error'], 'invalid', reference)
        for roots in ([], [str(other) + '/../Documents'], ['/'], 'x', [str(other)] * 9, [None]):
            self.assertEqual(self.capture(self.work / 'a.txt', roots)['error'], 'invalid', roots)
        self.assertTrue(self.capture(self.work / 'a.txt', [str(other), str(self.work)])['ok'])
        self.assertFalse((self.store() / self.attachment / 'b.txt').exists())

    def test_capture_refuses_symlinks_anywhere_in_the_path(self):
        secret = Path(self.tmp.name) / 'secret.txt'
        secret.write_text('secret')
        (self.work / 'file-link.txt').symlink_to(secret)
        (self.work / 'dir-link').symlink_to(self.tmp.name)
        (self.work / 'real').mkdir()
        (self.work / 'real' / 'inner.txt').write_text('inner')
        (self.work / 'real-link').symlink_to(self.work / 'real')
        for reference in (self.work / 'file-link.txt', self.work / 'dir-link' / 'secret.txt',
                          self.work / 'real-link' / 'inner.txt'):
            self.assertEqual(self.capture(reference)['error'], 'invalid', reference)
        self.assertFalse(self.store().exists() and any((self.store() / self.attachment).glob('*.txt')))
        # A link to the root itself is also refused, so the check cannot be bypassed by the root.
        link_root = Path(self.tmp.name) / 'root-link'
        link_root.symlink_to(self.work)
        self.assertEqual(self.capture(link_root / 'real' / 'inner.txt', [str(link_root)])['error'], 'invalid')

    def test_capture_distinguishes_missing_special_and_oversized_files(self):
        self.assertEqual(self.capture(self.work / 'missing.txt')['error'], 'not_found')
        self.assertEqual(self.capture(self.work / 'missing' / 'x.txt')['error'], 'not_found')
        os.mkfifo(self.work / 'pipe')
        self.assertEqual(self.capture(self.work / 'pipe')['error'], 'invalid')
        (self.work / 'folder').mkdir()
        self.assertEqual(self.capture(self.work / 'folder')['error'], 'invalid')
        (self.work / 'file.txt').write_text('x')
        self.assertEqual(self.capture(self.work / 'file.txt' / 'below')['error'], 'invalid')
        (self.work / 'big.bin').write_bytes(b'x' * 11)
        with patch.object(controller, 'MAX_BYTES', 10):
            self.assertEqual(self.capture(self.work / 'big.bin')['error'], 'too_large')
        self.assertEqual(list((self.store() / self.attachment).glob('*')) if self.store().exists() else [], [])

    def test_capture_bounds_a_file_that_grows_while_copied(self):
        source = self.work / 'grow.bin'
        source.write_bytes(b'x' * 8)
        with patch.object(controller, 'MAX_BYTES', 10), patch.object(controller, 'CHUNK', 4):
            info = os.stat(source)
            original = controller.os.fstat

            def growing(fd):
                result = original(fd)
                if result.st_ino == info.st_ino:
                    source.write_bytes(b'x' * 64)
                return result
            with patch.object(controller.os, 'fstat', growing):
                self.assertEqual(self.capture(source)['error'], 'too_large')
        self.assertEqual(list((self.store() / self.attachment).iterdir()), [])

    def test_capture_requires_a_file_owned_by_the_user(self):
        (self.work / 'file.txt').write_text('x')
        with patch.object(controller.os, 'geteuid', return_value=os.geteuid() + 1):
            self.assertIn(self.capture(self.work / 'file.txt')['error'], ('invalid', 'conflict'))

    def test_delete_attachment_removes_only_its_directory(self):
        other = str(uuid.uuid4())
        self.put()
        self.put(attachment=other)
        request = {'op': 'delete_attachment', 'project': None, 'conversation': self.conversation,
                   'attachment': self.attachment}
        self.assertEqual(self.call(request)[0]['result'], 'deleted')
        self.assertFalse((self.store() / self.attachment).exists())
        self.assertTrue((self.store() / other / 'note.txt').is_file())
        self.assertEqual(self.call(request)[0]['result'], 'deleted')

    def test_delete_conversation_and_project_purge_their_stores(self):
        project = str(uuid.uuid4())
        sibling = str(uuid.uuid4())
        self.project = project
        self.put()
        self.put(attachment=str(uuid.uuid4()), conversation=sibling)
        self.project = None
        self.put(attachment=str(uuid.uuid4()))
        request = {'op': 'delete_conversation', 'project': project, 'conversation': self.conversation}
        self.assertEqual(self.call(request)[0]['result'], 'deleted')
        self.assertFalse(self.store(project).exists())
        self.assertTrue(self.store(project, sibling).exists())
        self.assertEqual(self.call({'op': 'delete_project', 'project': project})[0]['result'], 'deleted')
        self.assertFalse((self.home / '.factory/attachments' / project).exists())
        self.assertTrue(self.store().exists())
        self.assertEqual(self.call({'op': 'delete_project', 'project': project})[0]['result'], 'deleted')
        self.assertEqual(self.error({'op': 'delete_project', 'project': 'chats'}), 'invalid')
        self.assertEqual(self.error({'op': 'delete_project', 'project': None}), 'invalid')

    def test_delete_without_any_store_succeeds(self):
        self.assertEqual(self.call({'op': 'delete_project', 'project': str(uuid.uuid4())})[0]['result'], 'deleted')
        self.assertEqual(self.call({'op': 'delete_conversation', 'project': None,
                                    'conversation': self.conversation})[0]['result'], 'deleted')

    def test_delete_never_follows_a_symlink_to_a_directory(self):
        outside = Path(self.tmp.name) / 'outside'
        outside.mkdir()
        (outside / 'keep.txt').write_text('keep')
        self.put()
        (self.store() / self.attachment).rename(self.store() / 'moved')
        (self.store() / self.attachment).symlink_to(outside)
        request = {'op': 'delete_attachment', 'project': None, 'conversation': self.conversation,
                   'attachment': self.attachment}
        self.assertEqual(self.error(request), 'conflict')
        self.assertEqual((outside / 'keep.txt').read_text(), 'keep')

    def test_prepare_worker_creates_private_per_attempt_outputs(self):
        project, attempt = str(uuid.uuid4()), str(uuid.uuid4())
        request = {'op': 'prepare_worker', 'project': project, 'conversation': self.conversation,
                   'attempt': attempt}
        for _ in range(2):
            self.assertEqual(self.call(request)[0]['result'], 'ready')
        outputs = self.store(project) / '.outputs' / attempt
        self.assertEqual(outputs.stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.error({**request, 'project': None}), 'invalid')
        self.assertEqual(self.error({**request, 'attempt': '../x'}), 'invalid')
        (outputs / 'report.html').write_text('<p>x</p>')
        self.assertEqual(self.call(request)[0]['result'], 'ready')
        self.assertTrue((outputs / 'report.html').exists())

    def test_responses_stay_bounded(self):
        with patch.object(controller, 'MAX_RESPONSE', 10):
            self.assertEqual(self.run_main({'op': 'delete_project', 'project': str(uuid.uuid4())}),
                             b'{"ok":false,"error":"unavailable"}\n')


if __name__ == '__main__':
    unittest.main()
