import { sql } from "drizzle-orm"
import {
  pgTable,
  text,
  integer,
  jsonb,
  primaryKey,
  foreignKey,
  uniqueIndex,
  check,
} from "drizzle-orm/pg-core"
import type { Run, RunEvent } from "@workspace/core"
export const runs = pgTable(
  "runs",
  {
    id: text().primaryKey(),
    rootAgentId: text().notNull(),
    version: integer().notNull(),
    status: text().notNull(),
    createdAt: text().notNull(),
    projection: jsonb().$type<Run>().notNull(),
  },
  (t) => [
    uniqueIndex("runs_one_active_root")
      .on(sql`(true)`)
      .where(
        sql`${t.status} in ('created','running','waiting-for-user','cancelling')`
      ),
    check("runs_version_positive", sql`${t.version} > 0`),
    check(
      "runs_projection_identity",
      sql`(${t.projection}->>'id' = ${t.id} and (${t.projection}->>'version')::integer = ${t.version} and ${t.projection}->>'status' = ${t.status} and ${t.projection}->>'rootAgentId' = ${t.rootAgentId}) is true`
    ),
    check(
      "runs_status_known",
      sql`${t.status} in ('created','running','waiting-for-user','cancelling','completed','failed','interrupted')`
    ),
  ]
)
export const agents = pgTable(
  "agents",
  {
    runId: text()
      .notNull()
      .references(() => runs.id, { onDelete: "cascade" }),
    id: text().notNull(),
    parentId: text(),
  },
  (t) => [
    primaryKey({ columns: [t.runId, t.id] }),
    foreignKey({
      columns: [t.runId, t.parentId],
      foreignColumns: [t.runId, t.id],
      name: "agents_parent_owned",
    }),
  ]
)
export const questions = pgTable(
  "questions",
  { runId: text().notNull(), id: text().notNull(), agentId: text().notNull() },
  (t) => [
    primaryKey({ columns: [t.runId, t.id] }),
    foreignKey({
      columns: [t.runId, t.agentId],
      foreignColumns: [agents.runId, agents.id],
      name: "questions_agent_owned",
    }),
  ]
)
export const permissions = pgTable(
  "permissions",
  { runId: text().notNull(), id: text().notNull(), agentId: text().notNull() },
  (t) => [
    primaryKey({ columns: [t.runId, t.id] }),
    foreignKey({
      columns: [t.runId, t.agentId],
      foreignColumns: [agents.runId, agents.id],
      name: "permissions_agent_owned",
    }),
  ]
)
export const messages = pgTable(
  "messages",
  { runId: text().notNull(), id: text().notNull(), agentId: text().notNull() },
  (t) => [
    primaryKey({ columns: [t.runId, t.id] }),
    foreignKey({
      columns: [t.runId, t.agentId],
      foreignColumns: [agents.runId, agents.id],
      name: "messages_agent_owned",
    }),
  ]
)
export const events = pgTable(
  "events",
  {
    runId: text()
      .notNull()
      .references(() => runs.id, { onDelete: "cascade" }),
    id: text().notNull(),
    sequence: integer().notNull(),
    event: jsonb().$type<RunEvent>().notNull(),
  },
  (t) => [
    primaryKey({ columns: [t.runId, t.id] }),
    uniqueIndex("events_sequence_unique").on(t.runId, t.sequence),
    check(
      "events_identity",
      sql`(${t.event}->>'id' = ${t.id} and ${t.event}->>'runId' = ${t.runId} and ${t.sequence} > 0) is true`
    ),
  ]
)
