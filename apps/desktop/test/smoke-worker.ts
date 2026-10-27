import { readFile } from "node:fs/promises"
import { createDatabase, verifyDatabaseTarget } from "@workspace/database"
import { databaseTargetSchema } from "../src/runtime/database.js"
import { DesktopHost } from "../src/runtime/host.js"
import { serveWorker } from "../src/runtime/worker-loop.js"
import { ObservationRelay } from "../src/runtime/observation-relay.js"
import { SyntheticRuntime } from "./synthetic-runtime.js"
const generation = process.argv[2],
  targetFile = process.argv[3],
  port = process.parentPort
if (!generation || !targetFile || !port)
  throw new Error("Synthetic worker requires exact fixture proof")
serveWorker(port, generation, async (send) => {
  const target = databaseTargetSchema.parse(
    JSON.parse(await readFile(targetFile, "utf8"))
  )
  if (!/^factory-test-[a-z0-9]+$/.test(target.project))
    throw new Error("Synthetic worker only accepts ephemeral fixtures")
  const helperUrl = new URL(
    "../../../../scripts/database/owned.mjs",
    import.meta.url
  ).href
  const helpers: { inspectTarget: (target: unknown) => unknown } = await import(
    /* @vite-ignore */ helperUrl
  )
  const verified = verifyDatabaseTarget(
    databaseTargetSchema.parse(helpers.inspectTarget(target))
  )
  const database = createDatabase(verified)
  const relay = new ObservationRelay((event) =>
    send({ type: "event", generation, event })
  )
  return new DesktopHost(database, new SyntheticRuntime(), relay.publish, () =>
    relay.close()
  )
})
