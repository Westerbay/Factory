import assert from "node:assert/strict"
import { mkdtemp, readFile, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import childProcess from "node:child_process"
import { syncBuiltinESMExports } from "node:module"
import { existsSync, mkdirSync, writeFileSync, symlinkSync } from "node:fs"
import { docker as runDocker } from "./docker.mjs"
import { prepare } from "./prepare.mjs"

test("Docker cleans buildx metadata on success and command failure without following links", async (t) => {
  const outside = await mkdtemp(join(tmpdir(), "factory-docker-outside-"))
  const sentinel = join(outside, "keep.txt")
  writeFileSync(sentinel, "preserve")
  let config
  let fail = false
  const command = t.mock.method(childProcess, "execFileSync", (_file, args) => {
    config = args[1]
    const buildx = join(config, "buildx")
    mkdirSync(buildx)
    writeFileSync(join(buildx, "activity"), "owned build metadata")
    symlinkSync(outside, join(config, "external"), "junction")
    if (fail) throw new Error("command failed with sensitive details")
    return "image built\n"
  })
  syncBuiltinESMExports()
  try {
    assert.equal(runDocker(["build"]), "image built")
    assert.equal(existsSync(config), false)
    assert.equal(await readFile(sentinel, "utf8"), "preserve")
    fail = true
    assert.throws(runDocker.bind(null, ["build"]), {
      message: "Local environment Docker operation failed or timed out",
    })
    assert.equal(existsSync(config), false)
    assert.equal(await readFile(sentinel, "utf8"), "preserve")
  } finally {
    command.mock.restore()
    syncBuiltinESMExports()
    await unlink(sentinel)
    await rmdir(outside)
  }
})

test("repeated preparation preserves installation and changes only the prepared image", async () => {
  const directory = await mkdtemp(join(tmpdir(), "factory-prepare-test-"))
  let image = "a"
  const commands = []
  const docker = (args) => {
    commands.push(args)
    if (args[0] === "build") return ""
    return JSON.stringify([
      {
        Id: `sha256:${image.repeat(64)}`,
        Os: "linux",
        Config: {
          User: "10000:10000",
          Labels: {
            "io.factory.environment.image": "2",
            "io.factory.codex.version": "0.160.0",
          },
        },
      },
    ])
  }
  try {
    const first = await prepare(directory, docker)
    image = "b"
    const second = await prepare(directory, docker)
    assert.equal(first.installationId, second.installationId)
    assert.notEqual(first.imageId, second.imageId)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
    assert.deepEqual(
      commands.map((args) => args[0]),
      ["build", "image", "build", "image"]
    )
    image = "invalid"
    await assert.rejects(prepare(directory, docker), /does not satisfy/)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
  } finally {
    await unlink(join(directory, "prepared.json"))
    await unlink(join(directory, "installation.json"))
    await rmdir(directory)
  }
})
