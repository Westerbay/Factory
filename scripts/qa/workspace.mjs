import { createHash } from "node:crypto"
import {
  lstat,
  mkdir,
  readFile,
  readdir,
  realpath,
  rmdir,
  unlink,
  writeFile,
} from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const checkout = resolve(fileURLToPath(new URL("../..", import.meta.url)))
const identity = createHash("sha256")
  .update(checkout)
  .digest("hex")
  .slice(0, 20)
const workspace = `/tmp/factory-qa-${identity}`
const skill = ".agents/skills/factory-acceptance/SKILL.md"
const directories = [
  "",
  ".agents",
  ".agents/skills",
  ".agents/skills/factory-acceptance",
]
const marker = `${JSON.stringify({ version: 1, checkout, workspace, uid: process.getuid() })}\n`
const contents = new Map([
  [".factory-qa-owner.json", marker],
  [
    "harmless.txt",
    await readFile(new URL("fixtures/harmless.txt", import.meta.url), "utf8"),
  ],
  [
    skill,
    await readFile(new URL("fixtures/SKILL.md", import.meta.url), "utf8"),
  ],
])

function refuse(reason) {
  throw new Error(`Refusing acceptance workspace operation: ${reason}`)
}

async function inspectOwnedTree() {
  // Check every directory before reading children; never follow fixture symlinks.
  for (const directory of directories) {
    const path = join(workspace, directory)
    const stat = await lstat(path)
    if (!stat.isDirectory() || stat.uid !== process.getuid())
      refuse(`unowned or non-directory path: ${path}`)
    if ((await realpath(path)) !== path) refuse(`noncanonical path: ${path}`)
    const expected = [
      ...directories
        .filter((entry) => entry && dirname(entry) === (directory || "."))
        .map((entry) => entry.split("/").at(-1)),
      ...[...contents.keys()]
        .filter((entry) => dirname(entry) === (directory || "."))
        .map((entry) => entry.split("/").at(-1)),
    ].sort()
    const actual = (await readdir(path)).sort()
    if (JSON.stringify(actual) !== JSON.stringify(expected))
      refuse(
        `unexpected or missing content in ${path}; preserve and inspect manually`
      )
  }
  for (const [file, content] of contents) {
    const path = join(workspace, file)
    const stat = await lstat(path)
    if (!stat.isFile() || stat.uid !== process.getuid() || stat.nlink !== 1)
      refuse(`unowned, linked or non-file path: ${path}`)
    if (
      stat.size !== Buffer.byteLength(content) ||
      (await readFile(path, "utf8")) !== content
    )
      refuse(
        `marker or generated content differs: ${path}; preserve and inspect manually`
      )
  }
}

async function main() {
  const args = process.argv.slice(2)
  // pnpm versions may forward the explicit argument separator.
  if (args[0] === "--") args.shift()
  const prepare = args.length === 1 && args[0] === "prepare"
  const cleanup =
    args.length === 2 && args[0] === "cleanup" && args[1] === "--agents-stopped"
  if (!prepare && !cleanup)
    refuse(
      "usage: pnpm qa:workspace -- prepare | pnpm qa:workspace -- cleanup --agents-stopped (explicit acknowledgement that all fixture agents are stopped)"
    )
  if (
    (await realpath(checkout)) !== checkout ||
    (await realpath("/tmp")) !== "/tmp"
  )
    refuse(
      "checkout and /tmp must be canonical paths without symlink ancestors"
    )
  let exists = true
  try {
    await lstat(workspace)
  } catch (error) {
    if (error.code !== "ENOENT") throw error
    exists = false
  }
  if (prepare && !exists) {
    // mkdir without recursive adoption; an existing path must pass the full guard.
    await mkdir(workspace, { mode: 0o700 })
    for (const directory of directories.slice(1))
      await mkdir(join(workspace, directory), { mode: 0o700 })
    for (const [file, content] of contents)
      await writeFile(join(workspace, file), content, {
        flag: "wx",
        mode: 0o600,
      })
  }
  if (!exists && cleanup) refuse(`workspace absent: ${workspace}`)
  await inspectOwnedTree()
  if (cleanup) {
    // No recursive removal: all entries are verified before any exact unlink/rmdir.
    for (const file of contents.keys()) await unlink(join(workspace, file))
    for (const directory of directories.toReversed())
      await rmdir(join(workspace, directory))
    console.log(
      JSON.stringify({ removed: workspace, database: "untouched" }, null, 2)
    )
    return
  }
  console.log(
    JSON.stringify(
      {
        workspacePath: workspace,
        skillPath: join(workspace, skill),
        skillName: "factory-acceptance",
        prompt:
          "Run the selected factory-acceptance skill. Ask its one question and wait for my answer; then use at most one native child for the bounded read-only harmless.txt task. Follow all fixture limits and report unavailable capabilities honestly.",
        cleanup: "pnpm qa:workspace -- cleanup --agents-stopped",
        notice:
          "Preparation performs no inference. Starting the task uses the existing signed-in account. Stop all fixture agents before cleanup; development database/history are retained.",
      },
      null,
      2
    )
  )
}

try {
  await main()
} catch (error) {
  console.error(error.message)
  process.exitCode = 1
}
