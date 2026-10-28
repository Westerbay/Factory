import assert from "node:assert/strict"
import { mkdtemp, readFile, unlink, rmdir } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { prepare } from "./prepare.mjs"

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
          Labels: { "io.factory.environment.image": "1" },
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
