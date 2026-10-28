import { readFile } from "node:fs/promises"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { createDatabase, verifyDatabaseTarget } from "@workspace/database"
import { z } from "zod"
/** Reuses the CLI's Docker inspection; no URL/env configuration or implicit provisioning. */
export async function openOwnedDatabase() {
  const root = resolve(fileURLToPath(new URL("../../../../", import.meta.url)))
  const owner = (
    await readFile(resolve(root, ".factory/database-owner"), "utf8")
  ).trim()
  const helperUrl = new URL(
    "../../../../scripts/database/owned.mjs",
    import.meta.url
  ).href
  const helpers: { inspectTarget: (target: unknown) => unknown } = await import(
    /* @vite-ignore */ helperUrl
  )
  const candidate = {
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
  // InspectTarget returns its verified database target; validate the trust boundary before composition.
  const proof = databaseTargetSchema.parse(helpers.inspectTarget(candidate))
  return createDatabase(verifyDatabaseTarget(proof))
}
export const databaseTargetSchema = z.strictObject({
  project: z.string(),
  owner: z.string(),
  directory: z.string(),
  database: z.string(),
  username: z.string(),
  password: z.string(),
  host: z.string(),
  port: z.number().int(),
  container: z.string(),
  labels: z.record(z.string(), z.string()),
  publishedHost: z.string(),
  publishedPort: z.number().int(),
})
