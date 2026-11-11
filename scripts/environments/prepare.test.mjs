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
  let diagnostic = false
  const command = t.mock.method(childProcess, "execFileSync", (_file, args) => {
    config = args[1]
    const buildx = join(config, "buildx")
    mkdirSync(buildx)
    writeFileSync(join(buildx, "activity"), "owned build metadata")
    symlinkSync(outside, join(config, "external"), "junction")
    if (fail) {
      const error = new Error("command failed with sensitive details")
      if (diagnostic)
        Object.assign(error, {
          code: "ETIMEDOUT",
          status: 137,
          signal: "SIGTERM",
          stderr: "private build output",
        })
      throw error
    }
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
    diagnostic = true
    assert.throws(runDocker.bind(null, ["build"]), {
      message:
        "Local environment Docker operation failed or timed out (code=ETIMEDOUT, status=137, signal=SIGTERM)",
    })
    assert.equal(existsSync(config), false)
  } finally {
    command.mock.restore()
    syncBuiltinESMExports()
    await unlink(sentinel)
    await rmdir(outside)
  }
})

test("repeated preparation preserves installation and changes only the prepared image", async () => {
  const directory = await mkdtemp(join(tmpdir(), "factory-prepare-test-"))
  const stageDirectory = await mkdtemp(join(directory, "candidate-"))
  let image = "a"
  let policy = "5"
  let claudeVersion = "2.1.284"
  let proxyFailure = false
  let workspaceStates = ""
  const commands = []
  const images = {
    workspace: `ghcr.io/example/factory-workspace@sha256:${"1".repeat(64)}`,
    egress: `ghcr.io/example/factory-egress@sha256:${"2".repeat(64)}`,
  }
  const docker = (args) => {
    commands.push(args)
    if (args[0] === "build") return ""
    if (args[0] === "container" && args[1] === "ls") {
      return args.includes("label=io.factory.environment")
        ? workspaceStates
        : ""
    }
    if (args[0] === "version")
      return JSON.stringify({ Os: "linux", ApiVersion: "1.56" })
    if (args[2].startsWith("factory-egress:") || args[2] === images.egress) {
      if (proxyFailure) throw new Error("fixture proxy failure")
      return JSON.stringify([
        {
          Id: `sha256:${"c".repeat(64)}`,
          Os: "linux",
          Config: {
            User: "0:0",
            Labels: {
              "io.factory.egress.image": "1",
              "io.factory.egress.squid": "7.7",
            },
          },
        },
      ])
    }
    return JSON.stringify([
      {
        Id: `sha256:${image.repeat(64)}`,
        Os: "linux",
        Config: {
          User: "10000:10000",
          Labels: {
            "io.factory.environment.image": policy,
            "io.factory.codex.version": "0.160.0",
            "io.factory.claude.version": claudeVersion,
          },
        },
      },
    ])
  }
  try {
    const checkOptions = { checkOnly: true }
    await prepare(directory, docker, images, checkOptions)
    for (const name of ["installation.json", "prepared.json", "egress.json"]) {
      assert.equal(existsSync(join(directory, name)), false)
    }
    commands.length = 0
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
      [
        "build",
        "image",
        "version",
        "build",
        "image",
        "container",
        "build",
        "image",
        "version",
        "build",
        "image",
        "container",
      ]
    )
    const before = commands.length
    assert.deepEqual(await prepare(directory, docker, images), second)
    const registryPreparation = commands.slice(before)
    assert.deepEqual(
      registryPreparation.filter(([command]) => command === "image"),
      [
        ["image", "inspect", images.workspace],
        ["image", "inspect", images.egress],
      ]
    )
    assert.equal(
      registryPreparation.some(([command]) => command === "build"),
      false
    )
    const beforePreflight = await readFile(
      join(directory, "prepared.json"),
      "utf8"
    )
    for (const state of ["running", "created", "paused", "restarting"]) {
      workspaceStates = `exited
${state}`
      await assert.rejects(
        prepare(directory, docker, images, checkOptions),
        /workspaces are active/
      )
    }
    workspaceStates = "exited"
    await prepare(directory, docker, images, checkOptions)
    assert.equal(
      await readFile(join(directory, "prepared.json"), "utf8"),
      beforePreflight
    )
    image = "d"
    workspaceStates = "running"
    const staged = await prepare(directory, docker, images, { stageDirectory })
    assert.equal(staged.imageId, `sha256:${"d".repeat(64)}`)
    assert.equal(staged.installationId, second.installationId)
    assert.deepEqual(
      JSON.parse(await readFile(join(stageDirectory, "prepared.json"), "utf8")),
      staged
    )
    assert.equal(
      await readFile(join(directory, "prepared.json"), "utf8"),
      beforePreflight
    )
    workspaceStates = "exited"
    await assert.rejects(
      prepare(directory, docker, images, checkOptions),
      /release changes the prepared workspace image or policy/
    )
    assert.equal(
      await readFile(join(directory, "prepared.json"), "utf8"),
      beforePreflight
    )
    proxyFailure = true
    image = "d"
    await assert.rejects(prepare(directory, docker), /fixture proxy failure/)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
    image = "invalid"
    await assert.rejects(prepare(directory, docker), /does not satisfy/)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
    image = "a"
    claudeVersion = "unexpected"
    await assert.rejects(prepare(directory, docker), /does not satisfy/)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
    claudeVersion = "2.1.284"
    policy = "3"
    await assert.rejects(prepare(directory, docker), /does not satisfy/)
    assert.deepEqual(
      JSON.parse(await readFile(join(directory, "prepared.json"), "utf8")),
      second
    )
  } finally {
    for (const name of ["prepared.json", "egress.json"]) {
      const path = join(stageDirectory, name)
      if (existsSync(path)) await unlink(path)
    }
    await rmdir(stageDirectory)
    await unlink(join(directory, "prepared.json"))
    await unlink(join(directory, "egress.json"))
    await unlink(join(directory, "installation.json"))
    await rmdir(directory)
  }
})
