"""Bounded attachment file operations in a user's owned, persistent Linux home.

Embedded in the API and run as `python3 -I -S -c`, so it uses only the standard
library. The first stdin line is a bounded JSON request; a `put` is followed by
length-prefixed frames. Every path is resolved from opened, non-symlink directory
descriptors. The caller owns authorization, environment fencing and classification.
"""
import ctypes
import errno
import hashlib
import json
import os
import shutil
import stat
import sys
import uuid

HOME = '/home/factory'
MAX_REQUEST = 8192
MAX_RESPONSE = 32768
MAX_BYTES = 52428800
MAX_READ = 4194304
MAX_FRAME = 1048576
MAX_HEAD = 8192
MAX_NAME = 255
MAX_REFERENCE = 1024
MAX_ROOTS = 8
CHUNK = 65536
PARTIAL = '.partial'
OUTPUTS = '.outputs'
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
RENAME_NOREPLACE = 1


class Problem(Exception):
    def __init__(self, code):
        self.code = code


def directory(parent, name, create=False):
    """Open a private directory owned by this user, creating it with mode 0700."""
    if create:
        try:
            os.mkdir(name, 0o700, dir_fd=parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIRECTORY_FLAGS, dir_fd=parent)
    info = os.fstat(fd)
    if info.st_uid != os.geteuid() or info.st_mode & 0o077:
        os.close(fd)
        raise Problem('conflict')
    return fd


def regular(parent, name):
    fd = os.open(name, FILE_FLAGS, dir_fd=parent)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
        os.close(fd)
        raise Problem('conflict')
    return fd


def rename_noreplace(directory_fd, source, destination):
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renameat2
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(directory_fd, os.fsencode(source), directory_fd, os.fsencode(destination), RENAME_NOREPLACE) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def parse_uuid(value):
    if not isinstance(value, str):
        raise Problem('invalid')
    try:
        parsed = uuid.UUID(value)
    except ValueError as error:
        raise Problem('invalid') from error
    if parsed.int == 0 or str(parsed) != value:
        raise Problem('invalid')
    return value


def optional_project(value):
    return None if value is None else parse_uuid(value)


def file_name(value):
    if (not isinstance(value, str) or not value or value in ('.', '..', PARTIAL)
            or len(value.encode('utf-8')) > MAX_NAME
            or any(c == '/' or ord(c) < 32 or 127 <= ord(c) < 160 for c in value)):
        raise Problem('invalid')
    return value


def bounded_int(value, maximum):
    if type(value) is not int or not 0 <= value <= maximum:
        raise Problem('invalid')
    return value


def check_fields(request, fields):
    if set(request) != {'op'} | fields:
        raise Problem('invalid')


def open_chain(root, names, create=False):
    """Open nested store directories; with `create` false, missing ones raise FileNotFoundError."""
    opened = []
    try:
        parent = root
        for name in names:
            parent = directory(parent, name, create)
            opened.append(parent)
        return opened.pop(), opened
    except BaseException:
        for fd in opened:
            os.close(fd)
        raise


def close_all(fds):
    for fd in fds:
        os.close(fd)


def conversation_names(project, conversation):
    return ['.factory', 'attachments', 'chats' if project is None else project, conversation]


class Output:
    """Bounded, hashed, fsynced write of one file followed by a no-replace rename."""

    def __init__(self, parent, name):
        self.parent = parent
        self.name = name
        self.fd = os.open(PARTIAL, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                          0o600, dir_fd=parent)
        self.size = 0
        self.digest = hashlib.sha256()
        self.head = bytearray()
        self.done = False

    def write(self, data):
        if self.size + len(data) > MAX_BYTES:
            raise Problem('too_large')
        view = memoryview(data)
        while view:
            written = os.write(self.fd, view)
            view = view[written:]
        self.size += len(data)
        self.digest.update(data)
        if len(self.head) < MAX_HEAD:
            self.head += data[:MAX_HEAD - len(self.head)]

    def finish(self):
        os.fsync(self.fd)
        os.close(self.fd)
        self.fd = None
        rename_noreplace(self.parent, PARTIAL, self.name)
        self.done = True
        os.fsync(self.parent)
        return {'size': self.size, 'sha256': self.digest.hexdigest(),
                'head': bytes(self.head).hex()}

    def abort(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        if not self.done:
            try:
                os.unlink(PARTIAL, dir_fd=self.parent)
            except FileNotFoundError:
                pass


def store(root, request, chunks):
    project = optional_project(request['project'])
    conversation = parse_uuid(request['conversation'])
    attachment = parse_uuid(request['attachment'])
    name = file_name(request['name'])
    conversation_fd, opened = open_chain(root, conversation_names(project, conversation), True)
    opened.append(conversation_fd)
    try:
        parent = directory(conversation_fd, attachment, True)
        opened.append(parent)
        output = Output(parent, name)
        try:
            for chunk in chunks():
                output.write(chunk)
            return output.finish()
        except BaseException:
            output.abort()
            raise
    finally:
        close_all(opened)


def stdin_exact(count):
    data = sys.stdin.buffer.read(count)
    if len(data) != count:
        raise Problem('invalid')
    return data


def framed_chunks():
    """Length-prefixed frames ended by an empty one; EOF before it is an abandoned upload."""
    while True:
        length = int.from_bytes(stdin_exact(4), 'big')
        if length == 0:
            return
        if length > MAX_FRAME:
            raise Problem('invalid')
        yield stdin_exact(length)


def put(root, request):
    check_fields(request, {'project', 'conversation', 'attachment', 'name'})
    return store(root, request, framed_chunks)


def components(path):
    if (not isinstance(path, str) or not path.startswith('/') or len(path) > MAX_REFERENCE
            or any(ord(c) < 32 or ord(c) == 127 for c in path)):
        raise Problem('invalid')
    parts = path.split('/')[1:]
    if not parts or any(part in ('', '.', '..') for part in parts):
        raise Problem('invalid')
    return parts


def open_source(reference, roots):
    parts = components(reference)
    if (not isinstance(roots, list) or not 1 <= len(roots) <= MAX_ROOTS
            or not any(len(parts) > len(root_parts) and parts[:len(root_parts)] == root_parts
                       for root_parts in map(components, roots))):
        raise Problem('invalid')
    parent = os.open('/', DIRECTORY_FLAGS)
    try:
        try:
            for part in parts[:-1]:
                child = os.open(part, DIRECTORY_FLAGS, dir_fd=parent)
                os.close(parent)
                parent = child
            fd = os.open(parts[-1], FILE_FLAGS, dir_fd=parent)
        except OSError as error:
            # A symlink in the path, or a non-directory component, is never followed.
            if error.errno in (errno.ELOOP, errno.ENOTDIR):
                raise Problem('invalid') from error
            raise
    finally:
        os.close(parent)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
        os.close(fd)
        raise Problem('invalid')
    if info.st_size > MAX_BYTES:
        os.close(fd)
        raise Problem('too_large')
    return fd


def capture(root, request):
    check_fields(request, {'project', 'conversation', 'attachment', 'name', 'reference', 'roots'})
    source = open_source(request['reference'], request['roots'])
    try:
        def chunks():
            # The file may grow while it is copied: `Output` enforces the bound.
            while True:
                data = os.read(source, CHUNK)
                if not data:
                    return
                yield data
        return store(root, request, chunks)
    finally:
        os.close(source)


def read(root, request):
    check_fields(request, {'project', 'conversation', 'attachment', 'name', 'offset', 'length'})
    project = optional_project(request['project'])
    conversation = parse_uuid(request['conversation'])
    attachment = parse_uuid(request['attachment'])
    name = file_name(request['name'])
    offset = bounded_int(request['offset'], MAX_BYTES)
    length = bounded_int(request['length'], MAX_READ)
    attachment_fd, opened = open_chain(root, conversation_names(project, conversation) + [attachment])
    opened.append(attachment_fd)
    try:
        fd = regular(attachment_fd, name)
    finally:
        close_all(opened)
    size = os.fstat(fd).st_size
    return fd, offset, max(0, min(length, size - offset))


def stream(fd, offset, length):
    try:
        out = sys.stdout.buffer
        out.write(json.dumps({'ok': True, 'result': {'length': length}}).encode() + b'\n')
        while length:
            data = os.pread(fd, min(CHUNK, length), offset)
            if not data:
                # The file shrank after the promised length was sent.
                out.flush()
                raise SystemExit(3)
            out.write(data)
            offset += len(data)
            length -= len(data)
        out.flush()
    finally:
        os.close(fd)


def remove(parent, name):
    """Delete a directory tree below an open descriptor; a missing one is already deleted."""
    if not shutil.rmtree.avoids_symlink_attacks:
        raise Problem('unavailable')
    try:
        info = os.stat(name, dir_fd=parent, follow_symlinks=False)
    except FileNotFoundError:
        return 'deleted'
    if not stat.S_ISDIR(info.st_mode):
        raise Problem('conflict')
    shutil.rmtree(name, dir_fd=parent)
    os.fsync(parent)
    return 'deleted'


def delete_in(root, names, target):
    try:
        parent, opened = open_chain(root, names)
    except FileNotFoundError:
        return 'deleted'
    opened.append(parent)
    try:
        return remove(parent, target)
    finally:
        close_all(opened)


def delete_attachment(root, request):
    check_fields(request, {'project', 'conversation', 'attachment'})
    project = optional_project(request['project'])
    conversation = parse_uuid(request['conversation'])
    return delete_in(root, conversation_names(project, conversation), parse_uuid(request['attachment']))


def delete_conversation(root, request):
    check_fields(request, {'project', 'conversation'})
    project = optional_project(request['project'])
    conversation = parse_uuid(request['conversation'])
    return delete_in(root, conversation_names(project, conversation)[:-1], conversation)


def delete_project(root, request):
    check_fields(request, {'project'})
    return delete_in(root, ['.factory', 'attachments'], parse_uuid(request['project']))


def prepare_worker(root, request):
    check_fields(request, {'project', 'conversation', 'attempt'})
    project = parse_uuid(request['project'])
    conversation = parse_uuid(request['conversation'])
    attempt = parse_uuid(request['attempt'])
    outputs, opened = open_chain(root, conversation_names(project, conversation) + [OUTPUTS, attempt], True)
    opened.append(outputs)
    close_all(opened)
    return 'ready'


OPERATIONS = {
    'put': put,
    'capture': capture,
    'delete_attachment': delete_attachment,
    'delete_conversation': delete_conversation,
    'delete_project': delete_project,
    'prepare_worker': prepare_worker,
}


def execute(request, home=HOME):
    """Run one non-streaming operation, or return the open file of a `read`."""
    if not isinstance(request, dict) or not isinstance(request.get('op'), str):
        raise Problem('invalid')
    op = request['op']
    if op != 'read' and op not in OPERATIONS:
        raise Problem('invalid')
    root = os.open(home, DIRECTORY_FLAGS)
    try:
        if op == 'read':
            return read(root, request)
        return OPERATIONS[op](root, request)
    finally:
        os.close(root)


def failure(error):
    if isinstance(error, Problem):
        return error.code
    if isinstance(error, FileNotFoundError):
        return 'not_found'
    if isinstance(error, (ValueError, TypeError, UnicodeError)):
        return 'invalid'
    if isinstance(error, (FileExistsError, NotADirectoryError, PermissionError, BlockingIOError)):
        return 'conflict'
    if isinstance(error, OSError) and error.errno in (errno.EEXIST, errno.ELOOP, errno.ENOTDIR):
        return 'conflict'
    return 'unavailable'


def main(home=HOME):
    try:
        line = sys.stdin.buffer.readline(MAX_REQUEST + 1)
        if not line.endswith(b'\n') or len(line) > MAX_REQUEST:
            raise Problem('invalid')
        result = execute(json.loads(line), home)
        response = {'ok': True, 'result': result}
    except (Problem, OSError, ValueError, TypeError, UnicodeError) as error:
        response = {'ok': False, 'error': failure(error)}
    if response['ok'] and isinstance(result, tuple):
        # Past this point a failure ends the process abnormally rather than
        # appending a second response to promised bytes.
        stream(*result)
        return
    encoded = json.dumps(response).encode()
    if len(encoded) > MAX_RESPONSE:
        encoded = b'{"ok":false,"error":"unavailable"}'
    sys.stdout.buffer.write(encoded + b'\n')
    sys.stdout.buffer.flush()


if __name__ == '__main__':
    if len(sys.argv) > 1:
        sys.stderr.write('Invalid Factory attachment controller arguments\n')
        raise SystemExit(2)
    main()
