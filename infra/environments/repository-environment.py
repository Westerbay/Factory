#!/usr/bin/python3
"""Prepare one checkout's tools and locked dependencies for isolated Feature workers."""

import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

CHECKOUT = Path("/workspace/checkout")
TOOLING = Path("/workspace/tooling")
SYSTEM_PATH = "/usr/local/bin:/usr/bin:/bin"
MISE = "/usr/local/bin/mise"
RUSTUP = "/opt/factory/rustup-bin/rustup"
BASE_RUST = Path("/opt/factory/rustup")
VERSION = re.compile(r"(?:[a-z][a-z0-9-]*-)?[0-9][0-9A-Za-z.+-]{0,63}")
TOOL_FILES = {
    "node": (".node-version", ".nvmrc"),
    "python": (".python-version",),
    "java": (".java-version",),
    "go": (".go-version",),
    "ruby": (".ruby-version",),
}
CORE_TOOLS = frozenset(TOOL_FILES)
TOOLS = CORE_TOOLS | {"rust", "pnpm", "npm", "yarn", "uv", "maven", "gradle", "jq", "ripgrep"}
INPUTS = (
    ".factory-tools.toml", "rust-toolchain.toml", "rust-toolchain", "Cargo.toml", "Cargo.lock",
    "package.json", "package-lock.json", "npm-shrinkwrap.json", "pnpm-lock.yaml",
    "pnpm-workspace.yaml", ".npmrc", "requirements.txt", "go.mod",
    *(name for names in TOOL_FILES.values() for name in names),
)


class PreparationError(Exception):
    """Only fixed, non-sensitive step names leave the preparation worker."""


def read_input(path):
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return None
    with os.fdopen(fd, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise PreparationError("configuration")
        data = stream.read(8 * 1024 * 1024 + 1)
    if len(data) > 8 * 1024 * 1024:
        raise PreparationError("configuration")
    return data


def version(value):
    if isinstance(value, str):
        value = value.strip().removeprefix("v")
        if VERSION.fullmatch(value) or value in {"stable", "nightly", "lts", "latest"} \
                or re.fullmatch(r"nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
            return value
    raise PreparationError("configuration")


def declared_tool(inputs, names):
    for name in names:
        if name in inputs:
            requested = inputs[name].decode().strip()
            return version("lts" if name == ".nvmrc" and requested == "lts/*" else requested)
    return None


class Repository:
    def __init__(self, checkout, tooling):
        inputs = {}
        digest = hashlib.sha256(b"factory-repository-environment-v2\0")
        digest.update(os.environ.get("FACTORY_WORKSPACE_IMAGE_ID", "").encode() + b"\0")
        for name in INPUTS:
            data = read_input(checkout / name)
            if data is not None:
                inputs[name] = data
                digest.update(name.encode() + b"\0" + hashlib.sha256(data).digest())
        self.checkout = checkout
        self.root = tooling / digest.hexdigest()
        self.tools = {}
        self.components = ()
        self.targets = ()
        self.rust_profile = "minimal"
        if "rust-toolchain.toml" in inputs:
            rust = tomllib.loads(inputs["rust-toolchain.toml"].decode())["toolchain"]
            self.tools["rust"] = version(rust["channel"])
            self.components = rust.get("components", ())
            self.targets = rust.get("targets", ())
            self.rust_profile = rust.get("profile", "minimal")
        elif "rust-toolchain" in inputs:
            self.tools["rust"] = version(inputs["rust-toolchain"].decode())
        elif "Cargo.toml" in inputs:
            self.tools["rust"] = "1.90.0"
        self.cargo = "Cargo.lock" in inputs
        self.python = "requirements.txt" in inputs

        for tool, names in TOOL_FILES.items():
            selected = declared_tool(inputs, names)
            if selected:
                self.tools[tool] = selected

        if "go" not in self.tools and "go.mod" in inputs:
            module = inputs["go.mod"].decode()
            release = re.search(r"(?m)^toolchain\s+go([0-9.]+)\s*$", module)
            if release is None:
                release = re.search(r"(?m)^go\s+([0-9.]+)\s*$", module)
            if release:
                self.tools["go"] = version(release[1])

        self.node_manager = None
        if "package.json" in inputs:
            package = json.loads(inputs["package.json"])
            manager = package.get("packageManager", "")
            if isinstance(manager, str) and "@" in manager:
                name, selected = manager.split("@", 1)
                if name in {"npm", "pnpm", "yarn"}:
                    self.tools[name] = version(selected.split("+", 1)[0])
            if "pnpm-lock.yaml" in inputs:
                self.node_manager = "pnpm"
            elif {"package-lock.json", "npm-shrinkwrap.json"} & inputs.keys():
                self.node_manager = "npm"

        if ".factory-tools.toml" in inputs:
            settings = tomllib.loads(inputs[".factory-tools.toml"].decode())
            declared = settings.get("tools")
            if set(settings) != {"tools"} or not isinstance(declared, dict) or declared.keys() - TOOLS:
                raise PreparationError("configuration")
            self.tools.update({tool: version(selected) for tool, selected in declared.items()})

    def uses_environment(self):
        return bool(self.tools or self.cargo or self.python or self.node_manager)

    def tool_path(self, name):
        return self.root / "tools" / name


def run_program(args, cwd, env, **kwargs):
    args = list(map(str, args))
    try:
        return subprocess.run(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL, check=False, **kwargs)
    except OSError as error:
        # The pnpm npm distribution has an intentional shebang-less POSIX shim.
        if error.errno != errno.ENOEXEC or Path(args[0]).name != "pnpm":
            raise
        executable = shutil.which(args[0], path=env["PATH"])
        if executable is None:
            raise
        return subprocess.run(
            ["/bin/sh", executable, *args[1:]], cwd=cwd, env=env,
            stdin=subprocess.DEVNULL, check=False, **kwargs,
        )


def command(args, cwd, env, step, output=None):
    # Installer output can include repository settings, package URLs or credentials.
    result = run_program(args, cwd, env,
                         stdout=output or subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if result.returncode:
        raise PreparationError(step)


def has_version(binary, selected, env):
    executable = shutil.which(binary, path=env["PATH"])
    if executable is None:
        return False
    result = run_program([executable, "--version"], "/tmp", env, capture_output=True, text=True)
    return result.returncode == 0 and re.search(
        rf"(?<![0-9.]){re.escape(selected)}(?![0-9])", result.stdout
    ) is not None


def install_tool(repository, name, selected, env):
    if name == "rust":
        rustup_env = dict(env, RUSTUP_HOME=str(BASE_RUST), RUSTUP_TOOLCHAIN=selected)
        base_matches = selected == "1.90.0" and repository.rust_profile == "minimal" \
            and not repository.targets and set(repository.components) <= {"clippy", "rustfmt"}
        if base_matches:
            return
        if repository.rust_profile not in {"minimal", "default", "complete"}:
            raise PreparationError("configuration")
        args = [RUSTUP, "toolchain", "install", selected, "--profile", repository.rust_profile]
        for option, values in (("--component", repository.components), ("--target", repository.targets)):
            if not isinstance(values, (tuple, list)) or len(values) > 32:
                raise PreparationError("configuration")
            for value in values:
                if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}", value):
                    raise PreparationError("configuration")
                args.extend((option, value))
        rustup_env.update(RUSTUP_HOME=str(repository.tool_path(name)),
                          CARGO_HOME=str(repository.root / "rustup-cargo"))
        command(args, "/tmp", rustup_env, "rust")
        return

    binary = {"python": "python3", "ripgrep": "rg", "maven": "mvn"}.get(name, name)
    if has_version(binary, selected, env):
        return
    destination = repository.tool_path(name)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if name in {"npm", "pnpm", "yarn"}:
        command(["npm", "install", "--prefix", destination, "--no-audit", "--no-fund",
                 "--ignore-scripts", f"{name}@{selected}"], "/tmp", env, name)
        return
    backend = f"core:{name}" if name in CORE_TOOLS else name
    requested = f"temurin-{selected}" if name == "java" and selected[0].isdigit() else selected
    command([MISE, "install-into", f"{backend}@{requested}", destination], "/tmp", env, name)


def tool_environment(repository, env):
    env = env.copy()
    paths = []
    for name in repository.tools:
        directory = repository.tool_path(name)
        if not directory.is_dir():
            continue
        if name == "rust":
            continue
        if name == "java":
            env["JAVA_HOME"] = str(directory)
        suffix = "node_modules/.bin" if name in {"npm", "pnpm", "yarn"} else "bin"
        paths.append(str(directory / suffix))
    if repository.python and (repository.root / "venv/bin").is_dir():
        paths.insert(0, str(repository.root / "venv/bin"))
    if "rust" in repository.tools:
        rustup = repository.tool_path("rust")
        env.update(
            RUSTUP_HOME=str(rustup if rustup.is_dir() else BASE_RUST),
            RUSTUP_TOOLCHAIN=repository.tools["rust"],
            RUSTUP_AUTO_INSTALL="0",
        )
    paths.append(env["PATH"])
    env["PATH"] = ":".join(paths)
    return env


def install_dependencies(repository, env):
    root, checkout = repository.root, repository.checkout
    if repository.python:
        venv = root / "venv"
        command(["python3", "-m", "venv", venv], "/tmp", env, "python")
        command([venv / "bin/pip", "--disable-pip-version-check", "install", "--only-binary=:all:",
                 "-r", checkout / "requirements.txt"], checkout, env, "python-dependencies")
    if repository.cargo:
        cargo_env = dict(env, CARGO_HOME=str(root / "cargo-downloads"))
        with (root / "cargo-config.toml").open("wb") as config:
            command(["cargo", "vendor", "--locked", "--versioned-dirs", root / "vendor"],
                    checkout, cargo_env, "cargo-dependencies", config)
        shutil.rmtree(root / "cargo-downloads", ignore_errors=True)
    if repository.node_manager:
        with tempfile.TemporaryDirectory(dir=root) as scratch:
            source = Path(scratch) / "source"
            shutil.copytree(checkout, source, symlinks=True, ignore=shutil.ignore_patterns(".git"))
            node_env = dict(env, XDG_CACHE_HOME=str(root / "node-cache"))
            command(node_install(repository, offline=False), source, node_env, "node-dependencies")


def node_install(repository, offline):
    root = repository.root
    if repository.node_manager == "pnpm":
        args = ["pnpm", "install", "--frozen-lockfile", "--ignore-scripts", "--ignore-pnpmfile",
                "--store-dir", root / "pnpm"]
        if not offline:
            # Both relay hops bound the number of open tunnels. Leave room for
            # idle connections while pnpm fetches metadata and tarballs.
            args.extend(("--network-concurrency", "8"))
    else:
        args = ["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund", "--logs-max=0",
                "--cache", root / "npm"]
    if offline:
        args.append("--offline")
    return args


def prepare(repository, env):
    if not repository.uses_environment():
        return
    root = repository.root
    root.mkdir(mode=0o700, exist_ok=True)
    lock = os.open(root.parent / (root.name + ".lock"), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock, "wb") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        if (root / "ready").is_file():
            return
        if root.exists():
            shutil.rmtree(root)
        root.mkdir(mode=0o700)
        try:
            env = {key: env[key] for key in (
                "HOME", "HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy",
                "NO_PROXY", "no_proxy",
            ) if key in env}
            env.update(PATH=SYSTEM_PATH, MISE_YES="1", MISE_CACHE_DIR="/tmp/mise-cache",
                       MISE_DATA_DIR="/tmp/mise-data", MISE_CONFIG_DIR="/tmp/mise-config",
                       MISE_PYTHON_COMPILE="0", NPM_CONFIG_REGISTRY="https://registry.npmjs.org",
                       PIP_INDEX_URL="https://pypi.org/simple/", PIP_NO_CACHE_DIR="1",
                       npm_config_package_import_method="copy", CARGO_HTTP_TIMEOUT="30")
            for name, selected in sorted(repository.tools.items(), key=lambda item: item[0] != "node"):
                install_tool(repository, name, selected, tool_environment(repository, env))
            install_dependencies(repository, tool_environment(repository, env))
            (root / "ready").touch(exist_ok=False)
        except BaseException:
            shutil.rmtree(root)
            raise


def activate(repository, writable, env):
    if not repository.uses_environment():
        return env
    root = repository.root
    if not (root / "ready").is_file():
        raise PreparationError("environment-not-prepared")
    env = tool_environment(repository, env)
    if repository.cargo:
        cargo_home = Path(env["HOME"]) / ".cargo"
        cargo_home.mkdir(exist_ok=True)
        shutil.copyfile(root / "cargo-config.toml", cargo_home / "config.toml")
        env.update(CARGO_HOME=str(cargo_home), CARGO_NET_OFFLINE="true")
    if repository.node_manager == "pnpm":
        cache = root / "node-cache"
        if cache.is_dir():
            node_cache = Path(env["HOME"]) / ".cache"
            shutil.copytree(cache, node_cache, dirs_exist_ok=True)
            env["XDG_CACHE_HOME"] = str(node_cache)
    if repository.node_manager and writable:
        install_env = dict(env, npm_config_package_import_method="copy")
        args = node_install(repository, offline=True)
        if repository.node_manager == "pnpm":
            help_text = run_program(["pnpm", "install", "--help"], "/tmp", env,
                                    capture_output=True, text=True).stdout
            if "--frozen-store" in help_text:
                args.append("--frozen-store")
                command(args, repository.checkout, install_env, "node-offline-install")
            else:
                # Older pnpm releases require a writable store even when offline.
                with tempfile.TemporaryDirectory(dir=repository.checkout) as store:
                    shutil.copytree(root / "pnpm", store, dirs_exist_ok=True)
                    args[args.index("--store-dir") + 1] = store
                    command(args, repository.checkout, install_env, "node-offline-install")
        else:
            command(args, repository.checkout, install_env, "node-offline-install")
        # The isolated checkout belongs to Factory. Never stage generated dependencies.
        exclude = repository.checkout / ".git/info/exclude"
        if exclude.is_file():
            with exclude.open("a") as stream:
                stream.write("\nnode_modules/\n")
    elif repository.node_manager == "pnpm":
        env.update(npm_config_store_dir=str(root / "pnpm"), npm_config_frozen_store="true")
    elif repository.node_manager == "npm":
        env["npm_config_cache"] = str(root / "npm")
    return env


if __name__ == "__main__":
    try:
        action, *args = sys.argv[1:]
        repository = Repository(CHECKOUT, TOOLING)
        if action == "prepare" and not args:
            prepare(repository, os.environ)
        elif action == "run" and len(args) >= 2 and args[0] in {"write", "read"}:
            env = activate(repository, args[0] == "write", os.environ)
            os.execve(args[1], args[1:], env)
        else:
            raise PreparationError("configuration")
    except PreparationError as error:
        print(f"Repository environment: {error}", file=sys.stderr, flush=True)
        sys.exit(1)
    except (KeyError, TypeError, ValueError, OSError):
        print("Repository environment: configuration", file=sys.stderr, flush=True)
        sys.exit(1)
