import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  cp,
  link,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

async function fixture(t) {
  const checkout = await mkdtemp("/tmp/factory-qa-command-test-")
  t.after(() => rm(checkout, { recursive: true, force: true }))
  await mkdir(join(checkout, "scripts"))
  await cp(
    fileURLToPath(new URL("../qa", import.meta.url)),
    join(checkout, "scripts/qa"),
    { recursive: true }
  )
  const run = (...args) => {
    const response = spawnSync(
      process.execPath,
      [join(checkout, "scripts/qa/workspace.mjs"), ...args],
      { encoding: "utf8" }
    )
    if (response.error) throw response.error
    return response
  }
  const prepared = run("prepare")
  assert.equal(prepared.status, 0, prepared.stderr)
  const result = JSON.parse(prepared.stdout)
  const workspace = result.workspacePath
  assert.match(workspace, /^\/tmp\/factory-qa-[a-f0-9]{20}$/)
  t.after(async () => {
    // Test owns both generated paths, including deliberately corrupted guard cases.
    await rm(workspace, { recursive: true, force: true })
  })
  const rejected = (...args) => {
    const response = run(...args)
    assert.equal(response.status, 1, response.stdout)
    assert.match(response.stderr, /Refusing acceptance workspace operation/)
    return response
  }
  return { checkout, workspace, run, rejected, result }
}

test("CLI safely reuses exact fixture and cleanup retains checkout skills and database marker", async (t) => {
  const { checkout, workspace, run, result } = await fixture(t)
  for (const file of [
    ".factory/database-owner",
    ".agents/skills/existing/SKILL.md",
  ]) {
    const path = join(checkout, file)
    await mkdir(join(path, ".."), { recursive: true })
    await writeFile(path, "existing user content")
  }
  assert.equal(
    run("--", "prepare").stdout,
    JSON.stringify(result, null, 2) + "\n"
  )
  const cleaned = run("--", "cleanup", "--agents-stopped")
  assert.equal(cleaned.status, 0, cleaned.stderr)
  await assert.rejects(readFile(join(workspace, "harmless.txt")), {
    code: "ENOENT",
  })
  for (const file of [
    ".factory/database-owner",
    ".agents/skills/existing/SKILL.md",
  ])
    assert.equal(
      await readFile(join(checkout, file), "utf8"),
      "existing user content"
    )
})

test("cleanup requires explicit stopped-work acknowledgement and refuses supplied paths", async (t) => {
  const { workspace, rejected } = await fixture(t)
  rejected("cleanup")
  rejected("cleanup", "--agents-stopped", "/tmp")
  rejected("prepare", "--path", "/tmp")
  assert.match(
    await readFile(join(workspace, "harmless.txt"), "utf8"),
    /Total objects: 5/
  )
})

test("unowned marker prevents preparation and cleanup without deleting generated files", async (t) => {
  const { workspace, rejected } = await fixture(t)
  const path = join(workspace, ".factory-qa-owner.json")
  const marker = JSON.parse(await readFile(path, "utf8"))
  marker.checkout = "/some-other-checkout"
  await writeFile(path, JSON.stringify(marker) + "\n")
  rejected("prepare")
  rejected("cleanup", "--agents-stopped")
  assert.match(
    await readFile(join(workspace, "harmless.txt"), "utf8"),
    /Total objects: 5/
  )
})

test("unexpected nested user file blocks every mutation", async (t) => {
  const { workspace, rejected } = await fixture(t)
  const user = join(workspace, ".agents/skills/factory-acceptance/notes.txt")
  await writeFile(user, "keep me")
  rejected("prepare")
  rejected("cleanup", "--agents-stopped")
  assert.equal(await readFile(user, "utf8"), "keep me")
  assert.match(
    await readFile(join(workspace, ".factory-qa-owner.json"), "utf8"),
    /checkout/
  )
})

test("modified or missing generated files block cleanup before deleting the marker", async (t) => {
  const { workspace, rejected } = await fixture(t)
  await writeFile(join(workspace, "harmless.txt"), "user changes")
  rejected("cleanup", "--agents-stopped")
  assert.equal(
    await readFile(join(workspace, "harmless.txt"), "utf8"),
    "user changes"
  )
  await rm(join(workspace, "harmless.txt"))
  rejected("cleanup", "--agents-stopped")
  assert.match(
    await readFile(join(workspace, ".factory-qa-owner.json"), "utf8"),
    /checkout/
  )
})

test("symlinked fixture root cannot adopt or delete another directory", async (t) => {
  const { checkout, workspace, rejected } = await fixture(t)
  await rm(workspace, { recursive: true })
  await symlink(checkout, workspace)
  rejected("prepare")
  rejected("cleanup", "--agents-stopped")
  assert.match(
    await readFile(join(checkout, "scripts/qa/workspace.mjs"), "utf8"),
    /inspectOwnedTree/
  )
})

test("symlinked directory or file is refused and linked target content survives", async (t) => {
  const { checkout, workspace, rejected } = await fixture(t)
  const target = join(checkout, "keep.txt")
  await writeFile(target, "keep me")
  await rm(join(workspace, "harmless.txt"))
  await symlink(target, join(workspace, "harmless.txt"))
  rejected("cleanup", "--agents-stopped")
  await rm(join(workspace, ".agents"), { recursive: true })
  await symlink(checkout, join(workspace, ".agents"))
  rejected("prepare")
  rejected("cleanup", "--agents-stopped")
  assert.equal(await readFile(target, "utf8"), "keep me")
})

test("hard-linked generated file prevents cleanup despite matching bytes", async (t) => {
  const { checkout, workspace, rejected } = await fixture(t)
  const target = join(checkout, "linked.txt")
  await link(join(workspace, "harmless.txt"), target)
  rejected("cleanup", "--agents-stopped")
  assert.match(await readFile(target, "utf8"), /Total objects: 5/)
})

test("an unmarked existing directory is never adopted or removed", async (t) => {
  const { workspace, rejected } = await fixture(t)
  await rm(join(workspace, ".factory-qa-owner.json"))
  rejected("prepare")
  rejected("cleanup", "--agents-stopped")
  assert.match(
    await readFile(join(workspace, "harmless.txt"), "utf8"),
    /Total objects: 5/
  )
})
