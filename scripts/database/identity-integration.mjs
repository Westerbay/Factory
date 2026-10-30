import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { mkdtemp, writeFile, unlink, rmdir } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { createDatabase } from "../../packages/database/dist/index.js"
import {
  cleanupFixture,
  compose,
  docker,
  inspectTarget,
  root,
} from "./owned.mjs"
import { runCargo, rustProcess } from "../rust/run.mjs"

function retainedEnvironment(targetPath) {
  const available = rustProcess().env
  const env = { FACTORY_TEST_TARGET_PATH: targetPath }
  for (const key of [
    "PATH",
    "SystemRoot",
    "USERPROFILE",
    "ProgramFiles",
    "HOME",
    "TEMP",
    "TMP",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "RUSTUP_HOME",
    "LIB",
    "INCLUDE",
    "LIBPATH",
  ]) {
    if (available[key] !== undefined) env[key] = available[key]
  }
  return env
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer()
    server.on("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port
      server.close(() => resolve(port))
    })
  })
}

function legacySnapshot(target) {
  inspectTarget(target)
  // Synthetic fixture data only. Preserve all legacy DDL, rows and Drizzle history.
  return docker([
    "exec",
    target.container,
    "pg_dump",
    "--no-owner",
    "--no-privileges",
    "--schema=public",
    "--schema=drizzle",
    "-U",
    "factory",
    target.database,
  ]).replace(/^\\(?:un)?restrict .*$/gm, "")
}

async function runFixture(populated) {
  const project = `factory-test-${randomUUID().replaceAll("-", "")}`
  const target = {
    project,
    owner: randomUUID(),
    directory: root,
    database: "factory_test",
    username: "factory",
    password: "local-fixture-only",
    host: "127.0.0.1",
    port: await freePort(),
    container: `${project}-postgres-1`,
  }
  const scratch = await mkdtemp(join(tmpdir(), "factory-identity-integration-"))
  const targetPath = join(scratch, "target.json")
  let created = false
  let database
  try {
    for (const args of [
      [
        "ps",
        "-a",
        "--filter",
        `name=^/${target.container}$`,
        "--format",
        "{{.ID}}",
      ],
      [
        "volume",
        "ls",
        "--filter",
        `name=^${project}_postgres_data$`,
        "--format",
        "{{.Name}}",
      ],
      [
        "network",
        "ls",
        "--filter",
        `name=^${project}_default$`,
        "--format",
        "{{.Name}}",
      ],
    ])
      assert.equal(docker(args), "", "fixture names must be unused")
    created = true
    compose(target, ["up", "-d", "--wait", "postgres"])
    const proof = inspectTarget(target)
    if (populated) {
      database = createDatabase(proof)
      await database.migrate()
      const run = {
        id: "legacy-preservation",
        rootAgentId: "root",
        workspacePath: "/fixture",
        providerId: "test",
        task: "Preserved legacy execution",
        status: "created",
        version: 1,
        createdAt: "2026-10-28T00:00:00.000Z",
        updatedAt: "2026-10-28T00:00:00.000Z",
        agents: [{ id: "root", status: "created" }],
        questions: [],
        permissions: [],
        messages: [],
      }
      await database.store.create(run, {
        id: "legacy-created",
        runId: run.id,
        type: "run-created",
        at: run.createdAt,
      })
      await database.close()
      database = undefined
    }
    const before = legacySnapshot(target)
    await writeFile(targetPath, `${JSON.stringify(target)}\n`, { flag: "wx" })
    for (const [packageName, suite] of [
      ["factory-database", "migrations"],
      ["factory-api", "composition"],
      ["factory-database", "identity"],
      ["factory-database", "environments"],
      ["factory-database", "environment_upgrades"],
    ]) {
      const result = runCargo(
        [
          "test",
          "--locked",
          "--offline",
          "--package",
          packageName,
          "--test",
          suite,
          "--",
          "--ignored",
          "--test-threads=1",
        ],
        { env: retainedEnvironment(targetPath) }
      )
      if (result.error || result.status !== 0)
        throw new Error(`Identity ${suite} integration failed`)
    }
    assert.equal(
      legacySnapshot(target),
      before,
      "legacy tables, rows, constraints and Drizzle ledger must remain unchanged"
    )
    console.log(
      `Identity integration passed with ${populated ? "populated legacy" : "fresh"} database`
    )
  } finally {
    await database?.close()
    if (created) cleanupFixture(target)
    await unlink(targetPath).catch((error) => {
      if (error.code !== "ENOENT") throw error
    })
    await rmdir(scratch)
  }
}

await runFixture(false)
await runFixture(true)
