import { mkdir, readFile, writeFile } from "node:fs/promises"
import { randomUUID } from "node:crypto"
import { resolve } from "node:path"
import {
  root,
  compose,
  inspectTarget,
  ownedVolume,
  ownedNetwork,
  docker,
} from "./owned.mjs"
import { createDatabase } from "../../packages/database/dist/index.js"
const action = process.argv[2]
if (!["setup", "upgrade", "reset", "status", "stop"].includes(action))
  throw new Error("Use setup, upgrade, reset, status or stop")
const directory = resolve(root, ".factory")
const marker = resolve(directory, "database-owner")
let owner
try {
  owner = (await readFile(marker, "utf8")).trim()
} catch (error) {
  if (error.code !== "ENOENT" || !["setup", "reset"].includes(action))
    throw new Error("Run pnpm db:setup to create an owned local database")
  // Before creating a new identity, reject any preexisting same-name resources.
  if (
    docker([
      "ps",
      "-a",
      "--filter",
      "name=^/factory-local-postgres-1$",
      "--format",
      "{{.ID}}",
    ]) ||
    docker([
      "volume",
      "ls",
      "--filter",
      "name=^factory-local_postgres_data$",
      "--format",
      "{{.Name}}",
    ]) ||
    docker([
      "network",
      "ls",
      "--filter",
      "name=^factory-local_default$",
      "--format",
      "{{.ID}}",
    ])
  )
    throw new Error("Refusing setup: unknown existing Factory resources")
  await mkdir(directory, { recursive: true })
  owner = randomUUID()
  await writeFile(marker, `${owner}\n`, { flag: "wx" })
}
const target = {
  project: "factory-local",
  owner,
  directory: root,
  database: "factory_local",
  username: "factory",
  password: "local-fixture-only",
  host: "127.0.0.1",
  port: 55432,
  container: "factory-local-postgres-1",
}
if (action === "setup" || action === "reset") {
  const existing = docker([
    "ps",
    "-a",
    "--filter",
    `name=^/${target.container}$`,
    "--format",
    "{{.ID}}",
  ])
  if (existing) {
    inspectTarget(target, { allowStopped: true })
    ownedVolume(target)
  } else if (
    docker([
      "volume",
      "ls",
      "--filter",
      "name=^factory-local_postgres_data$",
      "--format",
      "{{.Name}}",
    ])
  )
    ownedVolume(target)
  if (
    docker([
      "network",
      "ls",
      "--filter",
      "name=^factory-local_default$",
      "--format",
      "{{.ID}}",
    ])
  )
    ownedNetwork(target)
  compose(target, ["up", "-d", "--wait", "postgres"])
}
if (action === "stop") {
  inspectTarget(target)
  ownedVolume(target)
  compose(target, ["stop", "postgres"])
  console.log("Owned Factory PostgreSQL stopped; data retained")
} else {
  const database = createDatabase(inspectTarget(target))
  try {
    if (action === "reset") await database.reset()
    if (action === "upgrade") await database.upgradeFeatures()
    if (action === "setup") {
      const status = await database.ready()
      if (!status.available) await database.initialize()
    }
    console.log(JSON.stringify(await database.ready()))
  } finally {
    await database.close()
  }
}
