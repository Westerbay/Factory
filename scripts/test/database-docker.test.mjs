import assert from "node:assert/strict"
import childProcess from "node:child_process"
import { readFileSync } from "node:fs"
import { syncBuiltinESMExports } from "node:module"
import { resolve } from "node:path"
import test from "node:test"
import { compose, docker, root } from "../database/owned.mjs"

for (const platform of ["linux", "win32"]) {
  test(`Docker and Compose keep an explicit local target on ${platform}`, (t) => {
    const originalPlatform = Object.getOwnPropertyDescriptor(
      process,
      "platform"
    )
    const originalEnv = process.env
    Object.defineProperty(process, "platform", { value: platform })
    process.env = {
      PATH: originalEnv.PATH,
      SystemRoot: "C:\\Windows",
      USERPROFILE: "C:\\Users\\Factory User",
      ProgramFiles: "C:\\Program Files",
      DOCKER_HOST: "tcp://remote.invalid:2375",
      DOCKER_CONTEXT: "shared",
      COMPOSE_ENV_FILES: "private.env",
      FACTORY_DB_NAME: "shared",
    }
    const calls = []
    t.mock.method(childProcess, "execFileSync", (file, args, options) => {
      calls.push({ file, args, options })
      return " ready\n"
    })
    syncBuiltinESMExports()
    t.after(() => {
      Object.defineProperty(process, "platform", originalPlatform)
      process.env = originalEnv
      t.mock.restoreAll()
      syncBuiltinESMExports()
    })

    assert.equal(docker(["version"]), "ready")
    const target = {
      project: "factory-local",
      owner: "b52c14b8-a06e-48ee-ab1d-2194cda60568",
      directory: platform === "win32" ? "C:\\work\\Factory" : "/tmp/Factory",
      database: "factory_local",
      username: "factory",
      password: "local-fixture-only",
      host: "127.0.0.1",
      port: 55432,
      container: "factory-local-postgres-1",
    }
    compose(target, ["config", "--quiet"])
    const expectedHost =
      platform === "win32"
        ? "npipe:////./pipe/docker_engine"
        : "unix:///var/run/docker.sock"
    for (const { file, args, options } of calls) {
      assert.equal(file, "docker")
      assert.deepEqual(args.slice(0, 2), ["--host", expectedHost])
      assert.equal(options.env.DOCKER_HOST, undefined)
      assert.equal(options.env.DOCKER_CONTEXT, undefined)
      assert.equal(options.env.COMPOSE_ENV_FILES, undefined)
      assert.equal(options.shell, undefined)
      for (const name of ["SystemRoot", "USERPROFILE", "ProgramFiles"]) {
        assert.equal(
          options.env[name],
          platform === "win32" ? process.env[name] : undefined
        )
      }
    }
    assert.deepEqual(calls[1].args.slice(2), [
      "compose",
      "--env-file",
      resolve(root, "scripts/database/empty-env"),
      "-f",
      resolve(root, "compose.yml"),
      "-p",
      "factory-local",
      "config",
      "--quiet",
    ])
    assert.equal(calls[1].options.env.FACTORY_DB_DIRECTORY, target.directory)
    assert.equal(calls[1].options.env.FACTORY_DB_NAME, "factory_local")
    const envFile = readFileSync(calls[1].args[4], "utf8")
    assert.ok(
      envFile
        .split(/\r?\n/)
        .every((line) => !line.trim() || line.startsWith("#"))
    )

    assert.throws(
      () => compose({ ...target, host: "remote.invalid" }, ["up", "-d"]),
      /Refusing/
    )
    assert.equal(
      calls.length,
      2,
      "invalid targets must fail before spawning Docker"
    )
  })
}
