// Client lifecycle adapted from WebApp Template; see docs/REUSE.md and LICENSE.
import { drizzle } from "drizzle-orm/postgres-js"
import { migrate } from "drizzle-orm/postgres-js/migrator"
import { readMigrationFiles } from "drizzle-orm/migrator"
import { sql } from "drizzle-orm"
import postgres from "postgres"
import { isDeepStrictEqual } from "node:util"
import { fileURLToPath } from "node:url"
import { ExecutionError, reduceEvent, isTerminal } from "@workspace/core"
import type { Run, RunEvent, RunStore } from "@workspace/core"
import {
  runs,
  agents,
  questions,
  permissions,
  messages,
  events,
} from "./schema.js"
import { eq, and, asc, desc, gt } from "drizzle-orm"
import { verifyDatabaseTarget } from "./target.js"
import type { DatabaseTarget } from "./target.js"
const migrationsFolder = fileURLToPath(new URL("../drizzle", import.meta.url))
// dist and src sit at the same depth under packages/database.
export function createDatabase(proof: DatabaseTarget) {
  const target = verifyDatabaseTarget(proof)
  const client = postgres({
    host: target.host,
    port: target.port,
    database: target.database,
    username: target.username,
    password: target.password,
    max: 5,
    prepare: false,
    connect_timeout: 2,
    onnotice: () => {},
  })
  const db = drizzle(client)
  let closed = false
  async function safe<T>(operation: () => Promise<T>): Promise<T> {
    if (closed) throw new ExecutionError("persistence", "Database is closed")
    try {
      return await operation()
    } catch (error) {
      if (error instanceof ExecutionError) throw error
      // Driver causes can contain SQL, connection details or user content; keep them private.
      throw new ExecutionError("persistence", "Database operation failed")
    }
  }
  async function write(
    expectedVersion: number | undefined,
    run: Run,
    event: RunEvent
  ) {
    await safe(async () => {
      try {
        await db.transaction(async (tx) => {
          if (
            event.runId !== run.id ||
            run.version !== (expectedVersion ?? 0) + 1
          )
            throw new ExecutionError(
              "invalid-transition",
              "Inconsistent event version"
            )
          if (expectedVersion === undefined) {
            if (
              run.status !== "created" ||
              run.agents.length !== 1 ||
              run.agents[0]?.status !== "created" ||
              run.questions.length ||
              run.permissions.length ||
              run.messages.length
            )
              throw new ExecutionError(
                "invalid-transition",
                "Invalid initial execution"
              )
            if (event.type !== "run-created")
              throw new ExecutionError(
                "invalid-transition",
                "Expected creation event"
              )
            await tx.insert(runs).values({
              id: run.id,
              rootAgentId: run.rootAgentId,
              version: run.version,
              status: run.status,
              createdAt: run.createdAt,
              projection: run,
            })
          } else {
            const [current] = await tx
              .select()
              .from(runs)
              .where(eq(runs.id, run.id))
              .for("update")
            if (!current)
              throw new ExecutionError("not-found", "Execution not found")
            if (current.version !== expectedVersion)
              throw new ExecutionError(
                "invalid-transition",
                "Execution version changed"
              )
            if (!isDeepStrictEqual(reduceEvent(current.projection, event), run))
              throw new ExecutionError(
                "invalid-transition",
                "Projection does not match its event"
              )
            await tx
              .update(runs)
              .set({
                version: run.version,
                status: run.status,
                projection: run,
              })
              .where(eq(runs.id, run.id))
            await tx.delete(questions).where(eq(questions.runId, run.id))
            await tx.delete(permissions).where(eq(permissions.runId, run.id))
            await tx.delete(messages).where(eq(messages.runId, run.id))
            await tx.delete(agents).where(eq(agents.runId, run.id))
          }
          if (!run.agents.some((a) => a.id === run.rootAgentId && !a.parentId))
            throw new ExecutionError("ownership", "Root agent missing")
          if (run.agents.length)
            await tx.insert(agents).values(
              run.agents.map((a) => ({
                runId: run.id,
                id: a.id,
                parentId: a.parentId,
              }))
            )
          if (run.questions.length)
            await tx.insert(questions).values(
              run.questions.map((q) => ({
                runId: run.id,
                id: q.id,
                agentId: q.agentId,
              }))
            )
          if (run.permissions.length)
            await tx.insert(permissions).values(
              run.permissions.map((p) => ({
                runId: run.id,
                id: p.id,
                agentId: p.agentId,
              }))
            )
          if (run.messages.length)
            await tx.insert(messages).values(
              run.messages.map((m) => ({
                runId: run.id,
                id: m.id,
                agentId: m.agentId,
              }))
            )
          await tx.insert(events).values({
            runId: run.id,
            id: event.id,
            sequence: run.version,
            event,
          })
        })
      } catch (error) {
        // postgres-js errors can be wrapped by DrizzleQueryError. Inspect only codes/constraint names, never expose causes.
        let fault: unknown = error
        if (fault instanceof Error && fault.cause) fault = fault.cause
        if (typeof fault === "object" && fault !== null && "code" in fault) {
          if (fault.code === "23505") {
            if (
              "constraint_name" in fault &&
              fault.constraint_name === "runs_one_active_root"
            )
              throw new ExecutionError(
                "active-run",
                "An execution is already active"
              )
            throw new ExecutionError(
              "invalid-transition",
              "Duplicate execution or event"
            )
          }
          if (fault.code === "23503")
            throw new ExecutionError(
              "ownership",
              "Execution relation does not belong to its run"
            )
        }
        throw error
      }
    })
  }
  const store: RunStore = {
    create: (run, event) => write(undefined, run, event),
    commit: write,
    get: (runId) =>
      safe(async () => {
        const [row] = await db.select().from(runs).where(eq(runs.id, runId))
        return row?.projection
      }),
    getActive: () =>
      safe(async () => {
        const [row] = await db
          .select()
          .from(runs)
          .where(
            sql`${runs.status} in ('created','running','waiting-for-user','cancelling')`
          )
        return row?.projection
      }),
    findEvent: (runId, eventId) =>
      safe(async () => {
        const [row] = await db
          .select()
          .from(events)
          .where(and(eq(events.runId, runId), eq(events.id, eventId)))
        return row?.event
      }),
    list: (offset, limit) =>
      safe(async () => {
        page(offset, limit)
        const rows = await db
          .select()
          .from(runs)
          .orderBy(desc(runs.createdAt), asc(runs.id))
          .offset(offset)
          .limit(limit + 1)
        return {
          runs: rows.slice(0, limit).map((r) => r.projection),
          ...(rows.length > limit ? { nextOffset: offset + limit } : {}),
        }
      }),
    history: (runId, afterSequence, limit) =>
      safe(async () => {
        page(afterSequence, limit)
        const rows = await db
          .select()
          .from(events)
          .where(
            and(eq(events.runId, runId), gt(events.sequence, afterSequence))
          )
          .orderBy(asc(events.sequence))
          .limit(limit + 1)
        return {
          events: rows.slice(0, limit).map((r) => r.event),
          ...(rows.length > limit
            ? { nextSequence: rows[limit - 1]?.sequence }
            : {}),
        }
      }),
  }
  return {
    store,
    ready: async (): Promise<{
      available: boolean
      reason?: "unavailable" | "pending-migrations" | "closed"
    }> => {
      if (closed) return { available: false, reason: "closed" }
      try {
        const migrations = readMigrationFiles({ migrationsFolder })
        const rows = await client<
          { hash: string }[]
        >`select hash from drizzle.__drizzle_migrations order by created_at`
        if (
          rows.length !== migrations.length ||
          rows.some((r, i) => r.hash !== migrations[i]?.hash)
        )
          return { available: false, reason: "pending-migrations" }
        return { available: true }
      } catch (error) {
        if (
          typeof error === "object" &&
          error !== null &&
          "code" in error &&
          (error.code === "42P01" || error.code === "3F000")
        )
          return { available: false, reason: "pending-migrations" }
        return { available: false, reason: "unavailable" }
      }
    },
    migrate: () => safe(() => migrate(db, { migrationsFolder })),
    /** Explicit host startup step after migrations, before accepting starts. No tools are replayed. */
    recover: () =>
      safe(async () => {
        return db.transaction(async (tx) => {
          const active = await tx
            .select()
            .from(runs)
            .where(
              sql`${runs.status} in ('created','running','waiting-for-user','cancelling')`
            )
            .for("update")
          for (const row of active) {
            if (isTerminal(row.projection.status))
              throw new ExecutionError(
                "persistence",
                "Inconsistent persisted execution"
              )
            const event: RunEvent = {
              id: `recovery-${row.id}-${row.version}`,
              runId: row.id,
              at: new Date().toISOString(),
              type: "run-interrupted",
              reason: "Desktop runtime restarted; execution was not resumed",
            }
            const run = reduceEvent(row.projection, event)
            await tx
              .update(runs)
              .set({
                status: run.status,
                version: run.version,
                projection: run,
              })
              .where(eq(runs.id, row.id))
            await tx.insert(events).values({
              runId: row.id,
              id: event.id,
              sequence: run.version,
              event,
            })
          }
          return active.length
        })
      }),
    close: async () => {
      if (!closed) {
        closed = true
        await client.end({ timeout: 3 })
      }
    },
  }
}
function page(offset: number, limit: number) {
  if (
    !Number.isInteger(offset) ||
    offset < 0 ||
    !Number.isInteger(limit) ||
    limit < 1 ||
    limit > 200
  )
    throw new ExecutionError("invalid-transition", "Invalid history page")
}
