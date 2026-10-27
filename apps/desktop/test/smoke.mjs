import { createServer as createViteServer } from "vite"
import { loadConfigFromFile } from "electron-vite"
import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { createServer } from "node:net"
import { mkdtemp, mkdir, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { resolve, join } from "node:path"
import { spawn } from "node:child_process"
import { createRequire } from "node:module"
import { createDatabase } from "../../../packages/database/dist/index.js"
import {
  root,
  compose,
  inspectTarget,
  cleanupFixture,
  docker,
} from "../../../scripts/database/owned.mjs"
const artifacts = process.argv[2]
  ? resolve(process.argv[2])
  : await mkdtemp(join(tmpdir(), "factory-desktop-smoke-"))
await mkdir(artifacts, { recursive: true })
const project = `factory-test-${randomUUID().replaceAll("-", "")}`
const port = await new Promise((resolve, reject) => {
  const server = createServer()
  server.on("error", reject)
  server.listen(0, "127.0.0.1", () => {
    const address = server.address()
    server.close(() => resolve(address.port))
  })
})
const target = {
  project,
  owner: randomUUID(),
  directory: root,
  database: "factory_test",
  username: "factory",
  password: "local-fixture-only",
  host: "127.0.0.1",
  port,
  container: `${project}-postgres-1`,
}
const targetFile = join(artifacts, "fixture-target.json")
const require = createRequire(resolve(root, "apps/desktop/package.json"))
const electron = require("electron")
async function run(mode, rendererUrl = "") {
  await new Promise((resolveRun, reject) => {
    // No test provider switch exists in the production entry. This launches a separate test-only bundle.
    const child = spawn(
      electron,
      [
        resolve(root, "apps/desktop/out/smoke-main/smoke.js"),
        targetFile,
        artifacts,
        mode,
      ],
      {
        stdio: ["ignore", "pipe", "pipe"],
        env: { ...process.env, ELECTRON_RENDERER_URL: rendererUrl },
      }
    )
    let output = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error("Synthetic desktop process timed out"))
    }, 60_000)
    function capture(data) {
      output = (output + data.toString()).slice(-20_000)
    }
    child.stdout.on("data", capture)
    child.stderr.on("data", capture)
    child.on("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.on("exit", async (code, signal) => {
      clearTimeout(timer)
      await writeFile(join(artifacts, `electron-${mode}.log`), output)
      if (code !== 0)
        reject(
          new Error(
            `Synthetic desktop ${mode} failed (${code ?? signal}): ${output}`
          )
        )
      else {
        console.log(output.trim())
        resolveRun()
      }
    })
  })
}
let developmentServer
let provisioned = false,
  database
try {
  assert.equal(
    docker([
      "ps",
      "-a",
      "--filter",
      `name=^/${target.container}$`,
      "--format",
      "{{.ID}}",
    ]),
    ""
  )
  assert.equal(
    docker([
      "volume",
      "ls",
      "--filter",
      `name=^${project}_postgres_data$`,
      "--format",
      "{{.Name}}",
    ]),
    ""
  )
  provisioned = true
  compose(target, ["up", "-d", "--wait", "postgres"])
  const proof = inspectTarget(target)
  await writeFile(targetFile, JSON.stringify(proof))
  database = createDatabase(proof)
  await database.migrate()
  await database.close()
  database = undefined
  await run("journey")
  await run("recovery")
  const loaded = await loadConfigFromFile(
    { command: "serve", mode: "development" },
    resolve(root, "apps/desktop/electron.vite.config.ts")
  )
  developmentServer = await createViteServer({
    ...loaded.config.renderer,
    root: resolve(root, "apps/desktop/src/renderer"),
    configFile: false,
    server: { host: "127.0.0.1", port: 0 },
  })
  await developmentServer.listen()
  const developmentUrl = developmentServer.resolvedUrls.local[0]
  assert.ok(developmentUrl.startsWith("http://127.0.0.1:"))
  await run("development", developmentUrl)
  console.log(
    `Synthetic-only Electron acceptance passed. Screenshots/logs: ${artifacts}`
  )
} finally {
  await developmentServer?.close()
  await database?.close()
  if (provisioned) cleanupFixture(target)
}
