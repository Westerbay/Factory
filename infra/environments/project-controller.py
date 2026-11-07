#!/usr/bin/python3
"""Bounded project operations in a user's owned, persistent Linux home.

All paths are resolved relative to opened, non-symlink directory descriptors.
The caller owns authorization, environment fencing and durable operation receipts.
"""
import base64
import contextlib
import ctypes
import errno
import fcntl
import hashlib
import json
import os
import re
import selectors
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import time
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



def verify_feature_group(group, project_id, operation):
    for entry in os.listdir(group):
        try:
            pending = entry.startswith('.pending-')
            feature_id = parse_uuid(entry.removeprefix('.pending-'))
            feature = directory(group, entry, private=True)
            try:
                receipt = read_json_file(feature, 'feature', private=True)
            finally:
                os.close(feature)
            if (not isinstance(receipt, dict)
                    or set(receipt) != {'version', 'project', 'operation', 'feature', 'repository'}
                    or type(receipt['version']) is not int or receipt['version'] != 1
                    or receipt['project'] != project_id or receipt['operation'] != operation):
                raise Problem('conflict')
            recorded_feature = parse_uuid(receipt['feature'])
            if not pending and recorded_feature != feature_id:
                raise Problem('conflict')
            repository(receipt['repository'])
        except (FileNotFoundError, ValueError, Problem) as error:
            raise Problem('conflict') from error


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

            def resolve_private(item, group_name, source_name, validate):
                receipt_name = item + '-resolved'
                if resolved(receipt_name) is not None:
                    return
                with contextlib.ExitStack() as stack:
                    def opened_if_present(parent, child):
                        try:
                            fd = directory(parent, child, private=True)
                        except FileNotFoundError:
                            return None
                        stack.callback(os.close, fd)
                        return fd

                    staged = opened_if_present(record, item)
                    if staged is None:
                        group = opened_if_present(factory, group_name)
                        original = opened_if_present(group, source_name) if group is not None else None
                        if original is not None:
                            validate(original)
                            original_info = os.fstat(original)
                            rename_atomic(group, source_name, record, item)
                            os.fsync(group)
                            staged = opened_if_present(record, item)
                            if staged is None:
                                raise Problem('conflict')
                            moved_info = os.fstat(staged)
                            if (original_info.st_dev, original_info.st_ino) != (moved_info.st_dev, moved_info.st_ino):
                                raise Problem('conflict')
                    if staged is not None:
                        validate(staged)
                    mark_resolved(receipt_name, staged)

            def verify_import(staged):
                if read_json_file(staged, 'factory-operation', private=True) != expected:
                    raise Problem('conflict')

            resolve_private('features', 'features', project_id,
                            lambda group: verify_feature_group(group, project_id, operation))
            resolve_checkout()
            resolve_private('import', 'imports', operation, verify_import)
            for item in ('checkout', 'import', 'features'):
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


FEATURE_FIELDS = {
    'feature_initialize': set(),
    'feature_prepare': {'attempt', 'base_oid', 'writable'},
    'feature_checkpoint': {'attempt', 'base_oid', 'writable', 'scope'},
    'feature_integrate': {'attempt', 'base_oid', 'writable', 'scope', 'expected_head'},
    'feature_snapshot': set(),
    'feature_remote': set(),
    'feature_refresh': {'base_oid', 'head_oid', 'patch_digest'},
    'feature_push': {'base_oid', 'head_oid', 'patch_digest'},
}
FEATURE_ID_FIELDS = {'op', 'project', 'operation', 'directory', 'repository', 'feature'}
FEATURE_REQUEST_LIMIT = 192 * 1024
FEATURE_RESPONSE_LIMIT = 1024 * 1024
FEATURE_PATCH_LIMIT = 128 * 1024
FEATURE_PATH_LIMIT = 32 * 1024
PRE_PUSH = b"""#!/bin/sh
IFS=' ' read -r local_ref local_oid remote_ref remote_oid || exit 1
[ "$local_oid" = "$FACTORY_PUSH_OID" ] && [ "$remote_ref" = "$FACTORY_PUSH_REF" ] || exit 1
case "$remote_oid" in ''|*[!0]*) exit 1;; esac
if IFS= read -r extra; then exit 1; fi
"""


def fd_path(fd):
    return '/proc/self/fd/' + str(fd)


def oid(value):
    if not isinstance(value, str) or not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', value):
        raise Problem('invalid')
    return value


def feature_path(value, directory_scope=False):
    if not isinstance(value, str) or not value.strip() or len(value.encode()) > 1024:
        raise Problem('invalid')
    path = value.removesuffix('/') if directory_scope else value
    if '\\' in path or ':' in path or any(ord(c) < 32 or 127 <= ord(c) < 160 for c in path):
        raise Problem('invalid')
    if any(part in ('', '.', '..') or part.lower() == '.git' for part in path.split('/')):
        raise Problem('invalid')
    return value


def git_command(arguments, *, descriptors=(), repository_fd=None, extra_env=None, output_limit=8192,
                timeout=30, check=True, input_data=None):
    """Run only controller-selected Git commands with bounded, private output."""
    env = {
        'PATH': '/usr/bin:/bin',
        'HOME': '/nonexistent',
        'GIT_CONFIG_NOSYSTEM': '1',
        'GIT_CONFIG_GLOBAL': '/dev/null',
        'GIT_ATTR_NOSYSTEM': '1',
        'GIT_TERMINAL_PROMPT': '0',
        'GIT_SSH_COMMAND': '/usr/bin/ssh -F /etc/factory/github-ssh-config',
    }
    env.update(extra_env or {})
    argv = ['/usr/bin/git', '--no-pager', '-c', 'core.hooksPath=/dev/null',
            '-c', 'init.templateDir=/dev/null', '-c', 'maintenance.auto=false',
            '-c', 'gc.auto=0', '-c', 'protocol.ext.allow=never',
            '-c', 'core.quotePath=true']
    if repository_fd is not None:
        argv.extend(['--git-dir', fd_path(repository_fd)])
        descriptors = (*descriptors, repository_fd)
    with subprocess.Popen(
        [*argv, *arguments], stdin=subprocess.PIPE if input_data is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, pass_fds=tuple(set(descriptors)), cwd='/', env=env,
        start_new_session=True,
    ) as process:
        output = bytearray()
        deadline = time.monotonic() + timeout
        try:
            if input_data is not None:
                process.stdin.write(input_data)
                process.stdin.close()
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while True:
                    if not selector.select(max(0, deadline - time.monotonic())):
                        raise Problem('uncertain')
                    chunk = os.read(process.stdout.fileno(), min(65536, output_limit + 1 - len(output)))
                    if not chunk:
                        break
                    output.extend(chunk)
                    if len(output) > output_limit:
                        raise Problem('unavailable')
            status = process.wait(timeout=max(0, deadline - time.monotonic()))
        except (Problem, OSError, subprocess.TimeoutExpired) as error:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            if isinstance(error, subprocess.TimeoutExpired):
                raise Problem('uncertain') from error
            raise
    if check and status != 0:
        raise Problem('unavailable')
    return status, bytes(output)


def git_text(arguments, **kwargs):
    return git_command(arguments, **kwargs)[1].decode('utf-8').strip()


def get_ref(repo, ref):
    status, data = git_command(['rev-parse', '--verify', '--quiet', ref + '^{commit}'],
                               repository_fd=repo, check=False)
    if status == 1:
        return None
    if status != 0:
        raise Problem('unavailable')
    return oid(data.decode('ascii').strip())


def head_ref(value):
    if not isinstance(value, str) or not value.startswith('refs/heads/') or len(value.encode()) > 1024:
        raise Problem('conflict')
    git_command(['check-ref-format', value])
    return value


def remote_default(remote):
    lines = git_text(['ls-remote', '--symref', '--', remote, 'HEAD'], timeout=60).splitlines()
    symbolic, head = None, None
    for line in lines:
        value, ref = line.split('\t')
        if ref != 'HEAD':
            raise Problem('conflict')
        if value.startswith('ref: '):
            if symbolic is not None:
                raise Problem('conflict')
            symbolic = head_ref(value.removeprefix('ref: '))
        else:
            if head is not None:
                raise Problem('conflict')
            head = oid(value)
    if symbolic is None or head is None:
        raise Problem('conflict')
    return symbolic, head


def current_remote(remote, branch):
    refs = (branch['base_ref'], branch['head_ref'])
    values = {}
    for line in git_text(['ls-remote', '--refs', '--', remote, *refs], timeout=60).splitlines():
        value, ref = line.split('\t')
        if ref not in refs or ref in values:
            raise Problem('conflict')
        values[ref] = oid(value)
    return {'base_oid': values.get(refs[0]), 'head_oid': values.get(refs[1])}


def record_branch(record, remote, feature, initialize):
    head = 'refs/heads/factory/features/' + feature
    try:
        branch = read_json_file(record, 'branch', private=True)
    except FileNotFoundError:
        if not initialize:
            raise Problem('conflict')
        base_ref, _ = remote_default(remote)
        branch = {'base_ref': base_ref, 'head_ref': head}
        create_file(record, 'branch', json.dumps(branch).encode())
        os.fsync(record)
    fields = {'base_ref', 'head_ref'}
    if (not isinstance(branch, dict) or set(branch) not in (fields, fields | {'base_oid'})
            or branch['head_ref'] != head):
        raise Problem('conflict')
    head_ref(branch['base_ref'])
    if 'base_oid' in branch:
        # Older records stored the advertised OID before fetching. It cannot
        # establish the commit fetched into an unfinished private repository.
        oid(branch['base_oid'])
    return {'base_ref': branch['base_ref'], 'head_ref': head}


def initialize_repository(repo, remote, branch):
    try:
        head = regular(repo, 'HEAD')
    except FileNotFoundError:
        git_command(['init', '--bare', fd_path(repo)], descriptors=(repo,))
    else:
        os.close(head)
    current_head = get_ref(repo, branch['head_ref'])
    current_base = get_ref(repo, 'refs/factory/base')
    if current_head is not None:
        if current_base is None or not is_ancestor(repo, current_base, current_head):
            raise Problem('conflict')
        return
    git_command(['fetch', '--no-tags', '--no-recurse-submodules', '--', remote, branch['base_ref']],
                repository_fd=repo, timeout=240)
    fetched = get_ref(repo, 'FETCH_HEAD')
    if fetched is None:
        raise Problem('conflict')
    git_command(['symbolic-ref', 'HEAD', branch['head_ref']], repository_fd=repo)
    absent = '0' * len(fetched)
    updates = (f"start\nupdate refs/factory/base {fetched} {current_base or absent}\n"
               f"update {branch['head_ref']} {fetched} {absent}\nprepare\ncommit\n").encode()
    git_command(['update-ref', '--stdin'], repository_fd=repo, input_data=updates)


def feature_record(parent, name, receipt, expected, create):
    """Publish an owned record atomically, without adopting an unmarked directory."""
    try:
        result = directory(parent, name, private=True)
    except FileNotFoundError:
        if not create:
            raise Problem('conflict')
        staged = '.pending-' + str(uuid.uuid4())
        os.mkdir(staged, 0o700, dir_fd=parent)
        entry = directory(parent, staged, private=True)
        try:
            create_file(entry, receipt, json.dumps(expected).encode())
            os.fsync(entry)
            try:
                rename_atomic(parent, staged, parent, name)
            except FileExistsError:
                os.unlink(receipt, dir_fd=entry)
                os.rmdir(staged, dir_fd=parent)
            os.fsync(parent)
        finally:
            os.close(entry)
        result = directory(parent, name, private=True)
    try:
        if read_json_file(result, receipt, private=True) != expected:
            raise Problem('conflict')
        return result
    except FileNotFoundError as error:
        os.close(result)
        raise Problem('conflict') from error
    except BaseException:
        os.close(result)
        raise


def record_attempt(stack, record, request, prepare):
    attempt = parse_uuid(request['attempt'])
    base = oid(request['base_oid'])
    writable = request['writable']
    if not isinstance(writable, bool):
        raise Problem('invalid')
    attempts = directory(record, 'attempts', create=prepare, private=True)
    stack.callback(os.close, attempts)
    expected = {'version': 1, 'attempt': attempt, 'base': base, 'writable': writable}
    slot = feature_record(attempts, attempt, 'attempt', expected, prepare)
    stack.callback(os.close, slot)
    return slot, attempt, base


def prepare_feature_task(repo, slot, base):
    try:
        ready = read_json_file(slot, 'ready', private=True)
    except FileNotFoundError:
        if get_ref(repo, base) != base:
            raise Problem('conflict')
        if not shutil.rmtree.avoids_symlink_attacks:
            raise Problem('unavailable')
        try:
            shutil.rmtree('checkout', dir_fd=slot)
        except FileNotFoundError:
            pass
        checkout = directory(slot, 'checkout', create=True)
        try:
            git_command(['init', fd_path(checkout)], descriptors=(checkout,))
            git_command(['-C', fd_path(checkout), 'fetch', '--no-tags', '--no-recurse-submodules',
                         '--', fd_path(repo), base], descriptors=(repo, checkout), timeout=240)
            git_command(['-C', fd_path(checkout), 'checkout', '--detach', base],
                        descriptors=(checkout,), timeout=120)
            info = os.fstat(checkout)
            create_file(slot, 'ready', json.dumps({'device': info.st_dev, 'inode': info.st_ino}).encode())
            os.fsync(slot)
        finally:
            os.close(checkout)
    else:
        checkout = checked_checkout(slot, ready)
        os.close(checkout)


def checked_checkout(slot, ready):
    checkout = directory(slot, 'checkout')
    info = os.fstat(checkout)
    if ready != {'device': info.st_dev, 'inode': info.st_ino}:
        os.close(checkout)
        raise Problem('conflict')
    return checkout


def changed_feature_paths(repo, base, head, scope=None):
    output = git_command(['diff', '--raw', '--no-renames', '--no-ext-diff', '-z', base, head, '--'],
                         repository_fd=repo, output_limit=FEATURE_PATH_LIMIT)[1]
    if not output:
        return []
    fields = output.decode('utf-8').split('\0')
    if fields.pop() != '' or len(fields) % 2:
        raise Problem('unavailable')
    paths = []
    for details, path in zip(fields[::2], fields[1::2]):
        path = feature_path(path)
        if scope is not None:
            modes = details.removeprefix(':').split(' ', 2)[:2]
            if '160000' in modes or not any(item == path or item.endswith('/') and path.startswith(item)
                                             for item in scope):
                raise Problem('scope_violation')
        paths.append(path)
    return paths


def record_checkpoint(repo, slot, attempt, base, scope):
    checkout = checked_checkout(slot, read_json_file(slot, 'ready', private=True))
    try:
        # A nested repository can change git-add semantics. The controller reads
        # only the worker's files, with an index and config outside its mount.
        root = fd_path(checkout)
        for parent, directories, files in os.walk(root, followlinks=False):
            if parent == root:
                if '.git' in directories:
                    directories.remove('.git')
                continue
            if any(name.lower() == '.git' for name in (*directories, *files)):
                raise Problem('scope_violation')
        with tempfile.TemporaryDirectory(prefix='factory-feature-index-') as stage:
            env = {'GIT_INDEX_FILE': os.path.join(stage, 'index')}
            opts = ['--work-tree', root, '-c', 'core.bare=false', '-c', 'core.fsmonitor=false']
            kwargs = {'repository_fd': repo, 'descriptors': (checkout,), 'extra_env': env, 'timeout': 120}
            git_command([*opts, 'read-tree', base], **kwargs)
            git_command([*opts, 'add', '-A', '--', '.'], **kwargs)
            tree = oid(git_text([*opts, 'write-tree'], **kwargs))
    finally:
        os.close(checkout)
    paths = changed_feature_paths(repo, base, tree, scope)
    checkpoint_ref = 'refs/factory/attempts/' + attempt
    existing = get_ref(repo, checkpoint_ref)
    if existing is not None:
        if git_text(['rev-parse', existing + '^{tree}'], repository_fd=repo) != tree:
            raise Problem('conflict')
        return existing
    commit = base if not paths else feature_commit(repo, tree, (base,), 'Factory task ' + attempt)
    git_command(['update-ref', checkpoint_ref, commit, '0' * len(commit)], repository_fd=repo)
    return commit


def feature_commit(repo, tree, parents, message):
    timestamp = git_text(['show', '-s', '--format=%ct', parents[0]], repository_fd=repo)
    date = '@' + str(max(1, int(timestamp)) + 1) + ' +0000'
    env = {'GIT_AUTHOR_NAME': 'Factory', 'GIT_AUTHOR_EMAIL': 'factory@users.noreply.github.com',
           'GIT_COMMITTER_NAME': 'Factory', 'GIT_COMMITTER_EMAIL': 'factory@users.noreply.github.com',
           'GIT_AUTHOR_DATE': date, 'GIT_COMMITTER_DATE': date}
    parents = [item for parent in parents for item in ('-p', parent)]
    return oid(git_text(['commit-tree', tree, *parents, '-m', message], repository_fd=repo, extra_env=env))


def integrate_checkpoint(repo, branch, attempt, base, expected, scope):
    checkpoint = get_ref(repo, 'refs/factory/attempts/' + attempt)
    head = get_ref(repo, branch['head_ref'])
    if checkpoint is None or head is None:
        raise Problem('conflict')
    status, _ = git_command(['merge-base', '--is-ancestor', checkpoint, head],
                            repository_fd=repo, check=False)
    if status == 0:
        return head
    if status != 1 or head != expected:
        raise Problem('conflict')
    changed_feature_paths(repo, base, checkpoint, scope)
    if head == base:
        result = checkpoint
    else:
        status, output = git_command(['merge-tree', '--write-tree', '--no-messages', head, checkpoint],
                                     repository_fd=repo, check=False)
        if status != 0:
            raise Problem('conflict' if status == 1 else 'unavailable')
        tree = oid(output.decode('ascii').strip())
        changed_feature_paths(repo, head, tree, scope)
        result = feature_commit(repo, tree, (head, checkpoint), 'Integrate Factory task ' + attempt)
    git_command(['update-ref', branch['head_ref'], result, head], repository_fd=repo)
    return result


def feature_snapshot(repo, branch):
    base = branch['base_oid']
    head = get_ref(repo, branch['head_ref'])
    if head is None or not is_ancestor(repo, base, head):
        raise Problem('conflict')
    return snapshot_between(repo, base, head)


def snapshot_between(repo, base, head):
    patch = git_command(['diff', '--binary', '--full-index', '--no-color', '--no-renames',
                         '--no-ext-diff', '--no-textconv', '--src-prefix=a/', '--dst-prefix=b/',
                         base, head, '--'], repository_fd=repo, output_limit=FEATURE_PATCH_LIMIT)[1]
    return {'git': {'base_oid': base, 'head_oid': head, 'patch_digest': hashlib.sha256(patch).hexdigest()},
            'patch': patch.decode('utf-8'), 'changed_paths': changed_feature_paths(repo, base, head)}


def is_ancestor(repo, earlier, later):
    status, _ = git_command(['merge-base', '--is-ancestor', earlier, later], repository_fd=repo, check=False)
    if status not in (0, 1):
        raise Problem('unavailable')
    return status == 0


def refresh_feature_base(repo, remote, branch, expected):
    current = feature_snapshot(repo, branch)
    if current['git'] != expected:
        raise Problem('conflict')
    before = current_remote(remote, branch)
    old_base, old_head = expected['base_oid'], expected['head_oid']
    if before['base_oid'] == old_base:
        return current
    if before['head_oid'] is not None or before['base_oid'] is None:
        raise Problem('conflict')
    git_command(['fetch', '--no-tags', '--no-recurse-submodules', '--', remote, branch['base_ref']],
                repository_fd=repo, timeout=240)
    new_base = get_ref(repo, 'FETCH_HEAD')
    if new_base != before['base_oid'] or not is_ancestor(repo, old_base, new_base):
        raise Problem('conflict')
    if old_head == old_base:
        new_head = new_base
    else:
        status, output = git_command(['merge-tree', '--write-tree', '--no-messages', old_head, new_base],
                                     repository_fd=repo, check=False)
        if status != 0:
            raise Problem('conflict' if status == 1 else 'unavailable')
        tree = oid(output.decode('ascii').strip())
        new_head = feature_commit(repo, tree, (old_head, new_base), 'Refresh Factory feature base')
    updated = snapshot_between(repo, new_base, new_head)
    if current_remote(remote, branch) != before:
        raise Problem('conflict')
    # Controller readers share the feature lock. Git also compares both old
    # refs under its transaction before changing either.
    updates = (f"start\nupdate {branch['head_ref']} {new_head} {old_head}\n"
               f"update refs/factory/base {new_base} {old_base}\nprepare\ncommit\n").encode()
    git_command(['update-ref', '--stdin'], repository_fd=repo, input_data=updates)
    return updated


def publish_feature(record, repo, remote, branch, expected):
    if feature_snapshot(repo, branch)['git'] != expected:
        raise Problem('conflict')
    before = current_remote(remote, branch)
    if before['base_oid'] != expected['base_oid']:
        raise Problem('conflict')
    head, ref = expected['head_oid'], branch['head_ref']
    if before['head_oid'] == head:
        return 'pushed'
    if before['head_oid'] is not None:
        raise Problem('conflict')
    hooks = directory(record, 'push-hooks', create=True, private=True)
    try:
        try:
            create_file(hooks, 'pre-push', PRE_PUSH)
        except FileExistsError:
            pass
        hook = regular(hooks, 'pre-push', 0o077)
        try:
            if os.read(hook, len(PRE_PUSH) + 1) != PRE_PUSH:
                raise Problem('conflict')
            os.fchmod(hook, 0o500)
        finally:
            os.close(hook)
        status, _ = git_command(['-c', 'core.hooksPath=' + fd_path(hooks),
                                 'push', '--no-follow-tags', '--', remote, head + ':' + ref],
                                repository_fd=repo, descriptors=(hooks,), timeout=120, check=False,
                                extra_env={'FACTORY_PUSH_OID': head, 'FACTORY_PUSH_REF': ref})
    finally:
        os.close(hooks)
    after = current_remote(remote, branch)
    if after['base_oid'] != expected['base_oid'] or after['head_oid'] not in (None, head):
        raise Problem('conflict')
    if after['head_oid'] == head:
        return 'pushed'
    raise Problem('unavailable' if status != 0 else 'uncertain')


def execute_feature(request, root):
    op = request['op']
    if set(request) != FEATURE_ID_FIELDS | FEATURE_FIELDS[op]:
        raise Problem('invalid')
    project = parse_uuid(request['project'])
    operation = parse_uuid(request['operation'])
    feature = parse_uuid(request['feature'])
    remote = repository(request['repository'])
    initializing = op == 'feature_initialize'
    with contextlib.ExitStack() as stack:
        def opened(parent, name, create=False, private=True):
            fd = directory(parent, name, create=create, private=private)
            stack.callback(os.close, fd)
            return fd

        documents = opened(root, 'Documents', private=False)
        if inspect_project(documents, project, operation, request['directory']) != 'ready':
            raise Problem('conflict')
        factory = opened(root, '.factory', initializing)
        features = opened(factory, 'features', initializing)
        group = opened(features, project, initializing)
        expected = {'version': 1, 'project': project, 'operation': operation, 'feature': feature,
                    'repository': request['repository']}
        record = feature_record(group, feature, 'feature', expected, initializing)
        stack.callback(os.close, record)
        lock = os.open('lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                       0o600, dir_fd=record)
        stack.callback(os.close, lock)
        info = os.fstat(lock)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
            raise Problem('conflict')
        fcntl.flock(lock, fcntl.LOCK_EX)
        branch = record_branch(record, remote, feature, initializing)
        repo = opened(record, 'integration.git', initializing)
        if initializing:
            initialize_repository(repo, remote, branch)
        base = get_ref(repo, 'refs/factory/base')
        if base is None:
            raise Problem('conflict')
        branch['base_oid'] = base
        if initializing:
            return branch
        if op == 'feature_snapshot':
            return feature_snapshot(repo, branch)
        if op == 'feature_remote':
            return current_remote(remote, branch)
        if op == 'feature_refresh':
            return refresh_feature_base(repo, remote, branch,
                                        {key: request[key] for key in ('base_oid', 'head_oid', 'patch_digest')})
        if op == 'feature_push':
            return publish_feature(record, repo, remote, branch,
                                   {key: request[key] for key in ('base_oid', 'head_oid', 'patch_digest')})
        slot, attempt, base = record_attempt(stack, record, request, op == 'feature_prepare')
        if op == 'feature_prepare':
            prepare_feature_task(repo, slot, base)
            return 'ready'
        scope = request['scope']
        if not request['writable'] or not isinstance(scope, list) or not 1 <= len(scope) <= 128:
            raise Problem('invalid')
        scope = [feature_path(path, directory_scope=True) for path in scope]
        if op == 'feature_checkpoint':
            return record_checkpoint(repo, slot, attempt, base, scope)
        return integrate_checkpoint(repo, branch, attempt, base, oid(request['expected_head']), scope)


def execute(request, home=HOME):
    if not isinstance(request, dict) or not isinstance(request.get('op'), str):
        raise Problem('invalid')
    op = request['op']
    if op in FEATURE_FIELDS:
        root = os.open(home, DIRECTORY_FLAGS)
        try:
            return execute_feature(request, root)
        finally:
            os.close(root)
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
    response_limit = MAX_RESPONSE
    try:
        data = sys.stdin.buffer.read(FEATURE_REQUEST_LIMIT + 1)
        if len(data) > FEATURE_REQUEST_LIMIT:
            raise Problem('invalid')
        request = json.loads(data)
        if (isinstance(request, dict) and isinstance(request.get('op'), str)
                and request['op'] in FEATURE_FIELDS):
            response_limit = FEATURE_RESPONSE_LIMIT
        elif len(data) > MAX_REQUEST:
            raise Problem('invalid')
        result = execute(request)
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
    if len(encoded) <= response_limit:
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
