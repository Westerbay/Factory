#!/usr/bin/python3
"""Bounded project operations in a user's owned, persistent Linux home.

All paths are resolved relative to opened, non-symlink directory descriptors.
The caller owns authorization, environment fencing and durable operation receipts.
"""
import ctypes
import base64
import errno
import fcntl
import hashlib
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import uuid

HOME = '/home/factory'
MAX_REQUEST = 8192
MAX_RESPONSE = 8192
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
OWNER = re.compile(r'^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$')
REPO = re.compile(r'^[A-Za-z0-9_.-]{1,100}$')


class Problem(Exception):
    def __init__(self, code):
        self.code = code


def directory(parent, name, create=False, private=False):
    if create:
        try:
            os.mkdir(name, 0o700, dir_fd=parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIRECTORY_FLAGS, dir_fd=parent)
    info = os.fstat(fd)
    if info.st_uid != os.geteuid() or ((create or private) and info.st_mode & 0o077):
        os.close(fd)
        raise Problem('conflict')
    return fd


def regular(parent, name, mode=None):
    fd = os.open(name, FILE_FLAGS, dir_fd=parent)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or (mode is not None and info.st_mode & mode):
        os.close(fd)
        raise Problem('conflict')
    return fd


def read_json_file(parent, name, private=False):
    fd = regular(parent, name, 0o077 if private else None)
    try:
        data = os.read(fd, MAX_REQUEST + 1)
        if len(data) > MAX_REQUEST:
            raise Problem('conflict')
        return json.loads(data)
    finally:
        os.close(fd)


def create_file(parent, name, data):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=parent)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as output:
            output.write(data)
            output.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def rename_atomic(source_dir, source, destination_dir, destination, flags=1):
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renameat2
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(source_dir, os.fsencode(source), destination_dir, os.fsencode(destination), flags) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def identity(value):
    if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 320 or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise Problem('invalid')
    return value


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


def repository(name):
    if not isinstance(name, str) or name.count('/') != 1:
        raise Problem('invalid')
    owner, repo = name.split('/')
    if not OWNER.fullmatch(owner) or not REPO.fullmatch(repo) or repo in ('.', '..'):
        raise Problem('invalid')
    return 'git@github.com:' + owner + '/' + repo + '.git'


def public_key(active):
    fd = regular(active, 'github_ed25519', 0o077)
    try:
        result = subprocess.run(['/usr/bin/ssh-keygen', '-y', '-f', '/proc/self/fd/' + str(fd)],
                                pass_fds=(fd,), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=15, check=True)
    finally:
        os.close(fd)
    value = result.stdout.decode('ascii').strip()
    parts = value.split()
    if len(parts) < 2 or parts[0] != 'ssh-ed25519' or len(value) > 1024:
        raise Problem('conflict')
    return parts[0] + ' ' + parts[1]


def key_state(active):
    if read_json_file(active, 'factory-key', private=True) != {'version': 1}:
        raise Problem('conflict')
    public = public_key(active)
    fingerprint = hashlib.sha256(public.encode()).hexdigest()
    try:
        registered = read_json_file(active, 'registered', private=True)
    except FileNotFoundError:
        return {'public_key': public, 'registered': False}
    if registered != {'sha256': fingerprint}:
        raise Problem('conflict')
    return {'public_key': public, 'registered': True}


def bounded_file(parent, name, prohibited_mode=0o077):
    fd = regular(parent, name, prohibited_mode)
    try:
        data = os.read(fd, MAX_REQUEST + 1)
        if not data or len(data) > MAX_REQUEST:
            raise Problem('conflict')
        return data
    finally:
        os.close(fd)


def ssh_string(data, offset):
    if offset + 4 > len(data):
        raise Problem('conflict')
    length = struct.unpack_from('>I', data, offset)[0]
    start, end = offset + 4, offset + 4 + length
    if end > len(data):
        raise Problem('conflict')
    return data[start:end], end


def derive_ed25519_public(seed):
    # Debian bookworm's openssh-client already depends on this OpenSSL 3 library.
    # EVP derives a public key without running keygen or modifying the filesystem.
    crypto = ctypes.CDLL('libcrypto.so.3')
    crypto.EVP_PKEY_new_raw_private_key_ex.argtypes = [
        ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t,
    ]
    crypto.EVP_PKEY_new_raw_private_key_ex.restype = ctypes.c_void_p
    crypto.EVP_PKEY_get_raw_public_key.argtypes = [
        ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
    ]
    crypto.EVP_PKEY_get_raw_public_key.restype = ctypes.c_int
    crypto.EVP_PKEY_free.argtypes = [ctypes.c_void_p]
    crypto.EVP_PKEY_free.restype = None
    private = ctypes.create_string_buffer(seed, len(seed))
    key = None
    try:
        key = crypto.EVP_PKEY_new_raw_private_key_ex(None, b'ED25519', None, private, len(seed))
        if not key:
            raise Problem('conflict')
        output = ctypes.create_string_buffer(32)
        length = ctypes.c_size_t(len(output))
        if crypto.EVP_PKEY_get_raw_public_key(key, output, ctypes.byref(length)) != 1 or length.value != 32:
            raise Problem('conflict')
        return output.raw
    finally:
        if key:
            crypto.EVP_PKEY_free(key)
        ctypes.memset(private, 0, len(seed))


def registered_key_is_valid(active):
    """Inspect the stored key and receipt without running keygen or writing files."""
    try:
        if read_json_file(active, 'factory-key', private=True) != {'version': 1}:
            raise Problem('conflict')
        private = bounded_file(active, 'github_ed25519')
        lines = private.strip().splitlines()
        if len(lines) < 3 or lines[0] != b'-----BEGIN OPENSSH PRIVATE KEY-----' or lines[-1] != b'-----END OPENSSH PRIVATE KEY-----':
            raise Problem('conflict')
        data = base64.b64decode(b''.join(lines[1:-1]), validate=True)
        header = b'openssh-key-v1\x00'
        if not data.startswith(header):
            raise Problem('conflict')
        cipher, offset = ssh_string(data, len(header))
        kdf, offset = ssh_string(data, offset)
        options, offset = ssh_string(data, offset)
        if cipher != b'none' or kdf != b'none' or options or data[offset:offset + 4] != b'\x00\x00\x00\x01':
            raise Problem('conflict')
        public, offset = ssh_string(data, offset + 4)
        secret, offset = ssh_string(data, offset)
        if offset != len(data) or len(secret) < 8 or secret[:4] != secret[4:8]:
            raise Problem('conflict')
        algorithm, public_offset = ssh_string(public, 0)
        key_bytes, public_offset = ssh_string(public, public_offset)
        if algorithm != b'ssh-ed25519' or len(key_bytes) != 32 or public_offset != len(public):
            raise Problem('conflict')
        private_algorithm, secret_offset = ssh_string(secret, 8)
        private_public, secret_offset = ssh_string(secret, secret_offset)
        private_bytes, secret_offset = ssh_string(secret, secret_offset)
        _, secret_offset = ssh_string(secret, secret_offset)  # optional key comment
        padding = secret[secret_offset:]
        if (private_algorithm != algorithm or private_public != key_bytes or len(private_bytes) != 64
                or private_bytes[32:] != key_bytes or not padding
                or padding != bytes(range(1, len(padding) + 1))):
            raise Problem('conflict')
        if derive_ed25519_public(private_bytes[:32]) != key_bytes:
            raise Problem('conflict')
        public_file = bounded_file(active, 'github_ed25519.pub', 0o022).split()
        if len(public_file) < 2 or public_file[0] != algorithm or base64.b64decode(public_file[1], validate=True) != public:
            raise Problem('conflict')
        normalized = (algorithm + b' ' + base64.b64encode(public)).decode('ascii')
        try:
            receipt = read_json_file(active, 'registered', private=True)
        except FileNotFoundError:
            return False
        if receipt != {'sha256': hashlib.sha256(normalized.encode()).hexdigest()}:
            raise Problem('conflict')
        return True
    except FileNotFoundError as error:
        # An active directory cannot legitimately be missing its own key files.
        raise Problem('conflict') from error
    except (ValueError, UnicodeError, json.JSONDecodeError) as error:
        raise Problem('conflict') from error


def valid_git_identity(factory):
    try:
        fd = regular(factory, 'git-identity', 0o077)
    except FileNotFoundError:
        return False
    try:
        if os.fstat(fd).st_size > MAX_REQUEST:
            raise Problem('conflict')
        path = '/proc/self/fd/' + str(fd)
        values = []
        for field in ('user.name', 'user.email'):
            result = subprocess.run(['/usr/bin/git', 'config', '--no-includes', '--file', path, '--null', '--get', field],
                                    pass_fds=(fd,), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, timeout=5, check=False)
            if result.returncode != 0 or len(result.stdout) > 1024 or not result.stdout.endswith(b'\x00'):
                raise Problem('conflict')
            values.append(identity(result.stdout[:-1].decode('utf-8')))
        if not OWNER.fullmatch(values[0]) or '@' not in values[1]:
            raise Problem('conflict')
        return True
    except (ValueError, UnicodeError, Problem) as error:
        raise Problem('conflict') from error
    finally:
        os.close(fd)


def inspect_setup(root):
    try:
        factory = directory(root, '.factory', private=True)
    except FileNotFoundError:
        return False
    try:
        try:
            keys = directory(factory, 'ssh', private=True)
        except FileNotFoundError:
            return False
        try:
            try:
                lock = regular(keys, 'lock', 0o077)
            except FileNotFoundError:
                return False
            try:
                try:
                    # Writers already use this lock; opening it read-only never
                    # creates or alters a file. Do not claim readiness mid-rotation.
                    fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
                except BlockingIOError:
                    return False
                try:
                    active = directory(keys, 'active', private=True)
                except FileNotFoundError:
                    return False
                try:
                    if not registered_key_is_valid(active):
                        return False
                finally:
                    os.close(active)
            finally:
                os.close(lock)
        finally:
            os.close(keys)
        return valid_git_identity(factory)
    finally:
        os.close(factory)


def generate_candidate(keys):
    candidate = 'candidate-' + str(uuid.uuid4())
    os.mkdir(candidate, 0o700, dir_fd=keys)
    fd = directory(keys, candidate, private=True)
    try:
        path = '/proc/self/fd/' + str(fd) + '/github_ed25519'
        subprocess.run(['/usr/bin/ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-C', 'Factory', '-f', path],
                       pass_fds=(fd,), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=15, check=True)
        public_key(fd)
        private_key = regular(fd, 'github_ed25519', 0o077)
        try:
            os.fsync(private_key)
        finally:
            os.close(private_key)
        create_file(fd, 'factory-key', b'{"version":1}')
        os.fsync(fd)
        return candidate, fd
    except BaseException:
        os.close(fd)
        raise


def key(factory, action, expected=None):
    keys = directory(factory, 'ssh', True)
    try:
        lock = os.open('lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=keys)
        try:
            info = os.fstat(lock)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
                raise Problem('conflict')
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                active = directory(keys, 'active', private=True)
            except FileNotFoundError:
                if action != 'key':
                    raise Problem('conflict')
                candidate, candidate_fd = generate_candidate(keys)
                try:
                    rename_atomic(keys, candidate, keys, 'active')
                    os.fsync(keys)
                    return key_state(candidate_fd)
                finally:
                    os.close(candidate_fd)
            try:
                state = key_state(active)
                if action == 'key':
                    return state
                if not isinstance(expected, str) or expected != state['public_key']:
                    raise Problem('conflict')
                if action == 'mark':
                    if not state['registered']:
                        temporary = 'pending-registration-' + str(uuid.uuid4())
                        data = json.dumps({'sha256': hashlib.sha256(expected.encode()).hexdigest()}).encode()
                        create_file(active, temporary, data)
                        rename_atomic(active, temporary, active, 'registered')
                        os.fsync(active)
                    return key_state(active)
                if action != 'rotate' or not state['registered']:
                    raise Problem('conflict')
                # The caller must first confirm the registered key is absent on GitHub.
                # Exchanging whole directories atomically makes the new key and its
                # unregistered state visible together, even if this helper crashes.
                candidate, candidate_fd = generate_candidate(keys)
                try:
                    candidate_info = os.fstat(candidate_fd)
                    try:
                        rename_atomic(keys, 'active', keys, candidate, 2)  # RENAME_EXCHANGE
                        os.fsync(keys)
                        check = directory(keys, 'active', private=True)
                        try:
                            info = os.fstat(check)
                            if (info.st_dev, info.st_ino) != (candidate_info.st_dev, candidate_info.st_ino):
                                raise Problem('uncertain')
                            return key_state(check)
                        finally:
                            os.close(check)
                    except (OSError, Problem) as error:
                        raise Problem('uncertain') from error
                finally:
                    os.close(candidate_fd)
            finally:
                os.close(active)
        finally:
            os.close(lock)
    finally:
        os.close(keys)


def git_identity(factory, name, email):
    name, email = identity(name), identity(email)
    if not OWNER.fullmatch(name) or '@' not in email:
        raise Problem('invalid')
    # Git's system config includes this private, application-owned file. User global
    # and repository-local config retain their normal higher precedence.
    def quote(value):
        return value.replace('\\', '\\\\').replace('"', '\\"')
    data = ('[user]\n\tname = "' + quote(name) + '"\n\temail = "' + quote(email) + '"\n').encode()
    try:
        create_file(factory, 'git-identity', data)
    except FileExistsError:
        fd = regular(factory, 'git-identity', 0o077)
        os.close(fd)
    return 'configured'


def project_name(name):
    if not isinstance(name, str) or not REPO.fullmatch(name) or name in ('.', '..'):
        raise Problem('invalid')
    return name


def destination_status(documents, name):
    name = project_name(name)
    try:
        os.stat(name, dir_fd=documents, follow_symlinks=False)
    except FileNotFoundError:
        return 'available'
    return 'exists'


def marker(project_id, operation):
    return {'project': parse_uuid(project_id), 'operation': parse_uuid(operation)}


def inspect_project(documents, project_id, operation, name):
    name = project_name(name)
    try:
        path = directory(documents, name)
    except FileNotFoundError:
        return 'missing'
    try:
        metadata = directory(path, '.git')
        try:
            if read_json_file(metadata, 'factory-import') != marker(project_id, operation):
                raise Problem('conflict')
        finally:
            os.close(metadata)
    finally:
        os.close(path)
    return 'ready'


def inspect_import(factory, documents, project_id, operation, name):
    if inspect_project(documents, project_id, operation, name) == 'ready':
        return 'ready'
    try:
        imports = directory(factory, 'imports', private=True)
    except FileNotFoundError:
        return 'missing'
    try:
        try:
            stage = directory(imports, parse_uuid(operation), private=True)
        except FileNotFoundError:
            return 'missing'
        try:
            if read_json_file(stage, 'factory-operation', private=True) != marker(project_id, operation):
                raise Problem('conflict')
            # Presence of a receipt is not proof that its process has stopped.
            return 'staged'
        finally:
            os.close(stage)
    finally:
        os.close(imports)



def clean_import_stage(factory, project_id, operation):
    # Only a failed import with a verified stopped-environment observation may
    # call this. Keep its marker until the database has advanced to a new
    # operation so interrupted cleanup remains safely retryable.
    expected = marker(project_id, operation)
    if factory is None:
        return 'cleaned'
    if not shutil.rmtree.avoids_symlink_attacks:
        raise Problem('unavailable')
    try:
        imports = directory(factory, 'imports', private=True)
    except FileNotFoundError:
        return 'cleaned'
    try:
        try:
            stage = directory(imports, parse_uuid(operation), private=True)
        except FileNotFoundError:
            return 'cleaned'
        try:
            if read_json_file(stage, 'factory-operation', private=True) != expected:
                raise Problem('conflict')
            try:
                shutil.rmtree('checkout', dir_fd=stage)
            except FileNotFoundError:
                pass
            os.fsync(stage)
            return 'cleaned'
        finally:
            os.close(stage)
    finally:
        os.close(imports)


def clone(factory, documents, project_id, operation, full_name, name):
    name = project_name(name)
    receipt = marker(project_id, operation)
    remote = repository(full_name)
    try:
        os.stat(name, dir_fd=documents, follow_symlinks=False)
    except FileNotFoundError:
        pass
    else:
        raise Problem('conflict')
    imports = directory(factory, 'imports', True)
    try:
        # Never adopt or overwrite a pre-existing staging directory.
        os.mkdir(operation, 0o700, dir_fd=imports)
        stage = directory(imports, operation)
        try:
            create_file(stage, 'factory-operation', json.dumps(receipt).encode())
            target = '/proc/self/fd/' + str(stage) + '/checkout'
            env = {'HOME': HOME, 'PATH': '/usr/bin:/bin', 'GIT_TERMINAL_PROMPT': '0',
                   'GIT_SSH_COMMAND': '/usr/bin/ssh -F /etc/factory/github-ssh-config',
                   'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null'}
            # Avoid hanging indefinitely; timeout is deliberately uncertain to the caller.
            try:
                result = subprocess.run(['/usr/bin/git', '-c', 'init.templateDir=/dev/null', 'clone', '--', remote, target],
                                        pass_fds=(stage,), stdin=subprocess.DEVNULL,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                        env=env, timeout=240, check=False)
            except subprocess.TimeoutExpired as error:
                raise Problem('uncertain') from error
            if result.returncode != 0:
                # A failed Git process does not independently prove all of its
                # descendants stopped. Recovery requires an observed stop.
                raise Problem('uncertain')
            try:
                checkout = directory(stage, 'checkout')
                try:
                    checkout_info = os.fstat(checkout)
                    metadata = directory(checkout, '.git')
                    try:
                        create_file(metadata, 'factory-import', json.dumps(receipt).encode())
                        os.fsync(metadata)
                    finally:
                        os.close(metadata)
                    os.fsync(checkout)
                finally:
                    os.close(checkout)
                rename_atomic(stage, 'checkout', documents, name)
                os.fsync(stage)
                os.fsync(documents)
                final = directory(documents, name)
                try:
                    final_info = os.fstat(final)
                    if (final_info.st_dev, final_info.st_ino) != (checkout_info.st_dev, checkout_info.st_ino):
                        raise Problem('uncertain')
                finally:
                    os.close(final)
                if inspect_project(documents, project_id, operation, name) != 'ready':
                    raise Problem('uncertain')
            except (OSError, Problem) as error:
                raise Problem('uncertain') from error
            return 'ready'
        finally:
            os.close(stage)
    finally:
        os.close(imports)



def delete(factory, documents, project_id, operation, name):
    # Isolate the exact marked directory before recursive deletion. A receipt lives
    # independently of the DB so a retry cannot delete a newly created namesake.
    project_id = parse_uuid(project_id)
    operation = parse_uuid(operation)
    name = project_name(name)
    expected = marker(project_id, operation)
    if not shutil.rmtree.avoids_symlink_attacks:
        raise Problem('unavailable')
    deletions = directory(factory, 'deletions', True)
    try:
        record = directory(deletions, project_id, True)
        os.fsync(deletions)
        try:
            lock = os.open('lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=record)
        except BaseException:
            os.close(record)
            raise
        try:
            info = os.fstat(lock)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
                raise Problem('conflict')
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            receipt = {'project': project_id, 'operation': operation, 'directory': name}
            try:
                create_file(record, 'receipt', json.dumps(receipt).encode())
                os.fsync(record)
            except FileExistsError:
                if read_json_file(record, 'receipt', private=True) != receipt:
                    raise Problem('conflict')

            def resolved(key):
                try:
                    return read_json_file(record, key, private=True)
                except FileNotFoundError:
                    return None

            def mark_resolved(key, fd):
                info = os.fstat(fd) if fd is not None else None
                identity = [info.st_dev, info.st_ino] if info else None
                create_file(record, key, json.dumps({'identity': identity}).encode())
                os.fsync(record)

            def resolve_checkout():
                if resolved('checkout-resolved') is not None:
                    return
                try:
                    quarantined = directory(record, 'checkout')
                except FileNotFoundError:
                    quarantined = None
                if quarantined is None:
                    try:
                        original = directory(documents, name)
                    except FileNotFoundError:
                        original = None
                    if original is not None:
                        try:
                            original_info = os.fstat(original)
                            metadata = directory(original, '.git')
                            try:
                                if read_json_file(metadata, 'factory-import') != expected:
                                    raise Problem('conflict')
                            finally:
                                os.close(metadata)
                            rename_atomic(documents, name, record, 'checkout')
                            os.fsync(documents)
                            quarantined = directory(record, 'checkout')
                            moved_info = os.fstat(quarantined)
                            if (original_info.st_dev, original_info.st_ino) != (moved_info.st_dev, moved_info.st_ino):
                                os.close(quarantined)
                                raise Problem('conflict')
                        finally:
                            os.close(original)
                if quarantined is not None:
                    try:
                        metadata = directory(quarantined, '.git')
                        try:
                            if read_json_file(metadata, 'factory-import') != expected:
                                raise Problem('conflict')
                        finally:
                            os.close(metadata)
                        mark_resolved('checkout-resolved', quarantined)
                    finally:
                        os.close(quarantined)
                else:
                    mark_resolved('checkout-resolved', None)

            def resolve_import():
                if resolved('import-resolved') is not None:
                    return
                try:
                    imports = directory(factory, 'imports', private=True)
                except FileNotFoundError:
                    imports = None
                try:
                    try:
                        staged = directory(record, 'import', private=True)
                    except FileNotFoundError:
                        staged = None
                    if staged is None and imports is not None:
                        try:
                            original = directory(imports, operation, private=True)
                        except FileNotFoundError:
                            original = None
                        if original is not None:
                            try:
                                original_info = os.fstat(original)
                                if read_json_file(original, 'factory-operation', private=True) != expected:
                                    raise Problem('conflict')
                                rename_atomic(imports, operation, record, 'import')
                                os.fsync(imports)
                                staged = directory(record, 'import', private=True)
                                moved_info = os.fstat(staged)
                                if (original_info.st_dev, original_info.st_ino) != (moved_info.st_dev, moved_info.st_ino):
                                    os.close(staged)
                                    raise Problem('conflict')
                            finally:
                                os.close(original)
                    if staged is not None:
                        try:
                            if read_json_file(staged, 'factory-operation', private=True) != expected:
                                raise Problem('conflict')
                            mark_resolved('import-resolved', staged)
                        finally:
                            os.close(staged)
                    else:
                        mark_resolved('import-resolved', None)
                finally:
                    if imports is not None:
                        os.close(imports)

            resolve_checkout()
            resolve_import()
            for item in ('checkout', 'import'):
                receipt = resolved(item + '-resolved')
                try:
                    target = directory(record, item)
                except FileNotFoundError:
                    continue
                try:
                    info = os.fstat(target)
                    if not isinstance(receipt, dict) or receipt.get('identity') != [info.st_dev, info.st_ino]:
                        raise Problem('conflict')
                    # Keep an open descriptor for the verified inode while removing its
                    # contents; never follow a swapped directory or symlink.
                    for entry in os.listdir(target):
                        child = os.stat(entry, dir_fd=target, follow_symlinks=False)
                        if stat.S_ISDIR(child.st_mode):
                            shutil.rmtree(entry, dir_fd=target)
                        else:
                            os.unlink(entry, dir_fd=target)
                    current = os.stat(item, dir_fd=record, follow_symlinks=False)
                    if (current.st_dev, current.st_ino) != (info.st_dev, info.st_ino):
                        raise Problem('conflict')
                    os.rmdir(item, dir_fd=record)
                finally:
                    os.close(target)
            os.fsync(record)
            return 'deleted'
        finally:
            os.close(lock)
            os.close(record)
    finally:
        os.close(deletions)


def execute(request, home=HOME):
    if not isinstance(request, dict) or not isinstance(request.get('op'), str):
        raise Problem('invalid')
    op = request['op']
    fields = {'key': {'op'}, 'setup_status': {'op'}, 'mark': {'op', 'expected'}, 'rotate': {'op', 'expected'}, 'identity': {'op', 'name', 'email'},
              'delete': {'op', 'project', 'operation', 'directory'}, 'clean_stage': {'op', 'project', 'operation'}, 'destination_status': {'op', 'directory'}, 'inspect': {'op', 'project', 'operation', 'directory'}, 'clone': {'op', 'project', 'operation', 'repository', 'directory'}}
    if op not in fields or set(request) != fields[op]:
        raise Problem('invalid')
    root = os.open(home, DIRECTORY_FLAGS)
    try:
        if op == 'setup_status':
            return inspect_setup(root)
        if op in ('key', 'mark', 'rotate', 'identity', 'clone', 'delete'):
            factory = directory(root, '.factory', True)
        else:
            try:
                factory = directory(root, '.factory', private=True)
            except FileNotFoundError:
                factory = None
        try:
            if op == 'key':
                return key(factory, 'key')
            if op == 'mark':
                return key(factory, 'mark', request['expected'])
            if op == 'rotate':
                return key(factory, 'rotate', request['expected'])
            if op == 'identity':
                return git_identity(factory, request['name'], request['email'])
            if op == 'clean_stage':
                return clean_import_stage(factory, request['project'], request['operation'])
            documents = directory(root, 'Documents')
            try:
                if op == 'delete':
                    return delete(factory, documents, request['project'], request['operation'], request['directory'])
                if op == 'destination_status':
                    return destination_status(documents, request['directory'])
                if op == 'inspect':
                    if factory is None:
                        return inspect_project(documents, request['project'], request['operation'], request['directory'])
                    return inspect_import(factory, documents, request['project'], request['operation'], request['directory'])
                return clone(factory, documents, request['project'], request['operation'], request['repository'], request['directory'])
            finally:
                os.close(documents)
        finally:
            if factory is not None:
                os.close(factory)
    finally:
        os.close(root)


def main():
    try:
        data = sys.stdin.buffer.read(MAX_REQUEST + 1)
        if len(data) > MAX_REQUEST:
            raise Problem('invalid')
        result = execute(json.loads(data))
        response = {'ok': True, 'result': result}
    except Problem as error:
        response = {'ok': False, 'error': error.code}
    except (ValueError, TypeError, UnicodeError, json.JSONDecodeError):
        response = {'ok': False, 'error': 'invalid'}
    except (FileExistsError, NotADirectoryError, PermissionError, BlockingIOError):
        response = {'ok': False, 'error': 'conflict'}
    except OSError as error:
        response = {'ok': False, 'error': 'conflict' if error.errno in (errno.EEXIST, errno.ELOOP, errno.ENOTDIR) else 'unavailable'}
    except (subprocess.SubprocessError, AttributeError):
        response = {'ok': False, 'error': 'unavailable'}
    encoded = json.dumps(response).encode()
    if len(encoded) <= MAX_RESPONSE:
        sys.stdout.buffer.write(encoded + b'\n')
    else:
        sys.stdout.buffer.write(b'{"ok":false,"error":"unavailable"}\n')


def bootstrap():
    """Initialize the persistent key before becoming the container's idle process."""
    try:
        # Never print the result: it contains the public key and is not a log record.
        execute({'op': 'key'})
        os.execv('/bin/sleep', ['/bin/sleep', 'infinity'])
    except Exception:
        # Keep boot failures visible without leaking filesystem or key details.
        sys.stderr.write('Factory workspace SSH initialization failed\n')
        raise SystemExit(1)


if __name__ == '__main__':
    if sys.argv[1:] == ['--bootstrap']:
        bootstrap()
    elif len(sys.argv) == 1:
        main()
    else:
        sys.stderr.write('Invalid Factory project controller arguments\n')
        raise SystemExit(2)
