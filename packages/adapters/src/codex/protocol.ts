import { z } from "zod"
import type { ToolRequestUserInputParams } from "./generated/v2/ToolRequestUserInputParams.js"
import type { ServerRequestResolvedNotification } from "./generated/v2/ServerRequestResolvedNotification.js"

// Validated projections of Codex 0.160.0 generated protocol; unrelated fields are stripped.
export const PINNED_CODEX_VERSION = "0.160.0"
export const identity = z.object({
  threadId: z.string().min(1),
  turnId: z.string().min(1),
})
export const threadResult = z.object({
  thread: z.object({ id: z.string().min(1) }),
})
export const turnResult = z.object({
  turn: z.object({
    id: z.string().min(1),
    status: z.enum(["completed", "interrupted", "failed", "inProgress"]),
  }),
})
export const initializeResult = z.object({
  userAgent: z.string(),
  codexHome: z.string(),
  platformFamily: z.string(),
  platformOs: z.string(),
})
export const accountResult = z.object({
  account: z.object({ type: z.string() }).nullable(),
  requiresOpenaiAuth: z.boolean(),
})
export const skillsResult = z.object({
  data: z.array(
    z.object({
      cwd: z.string(),
      skills: z.array(
        z.object({
          name: z.string().min(1),
          path: z.string().min(1),
          enabled: z.boolean(),
        })
      ),
      errors: z.array(z.unknown()),
    })
  ),
})
export const questionParams = identity.extend({
  itemId: z.string(),
  isBlocking: z.boolean(),
  autoResolutionMs: z.number().nullable(),
  questions: z
    .array(
      z.object({
        id: z.string().min(1),
        header: z.string(),
        question: z.string().min(1),
        isOther: z.boolean(),
        isSecret: z.boolean(),
        options: z
          .array(z.object({ label: z.string(), description: z.string() }))
          .nullable(),
      })
    )
    .min(1)
    .max(20),
}) satisfies z.ZodType<ToolRequestUserInputParams>
export const permissionParams = identity.extend({
  itemId: z.string(),
  kind: z.enum(["command", "writeStdin"]).optional(),
  grantRoot: z.string().nullable().optional(),
  additionalPermissions: z.unknown().optional(),
  networkApprovalContext: z.unknown().optional(),
  reason: z.string().max(4000).nullable().optional(),
  command: z.string().max(16000).nullable().optional(),
  cwd: z.string().max(4000).nullable().optional(),
  availableDecisions: z.array(z.unknown()).nullable().optional(),
})
export const resolvedParams = z.object({
  threadId: z.string(),
  requestId: z.union([z.string(), z.number().int()]),
}) satisfies z.ZodType<ServerRequestResolvedNotification>
export const deltaParams = identity.extend({
  itemId: z.string(),
  delta: z.string(),
})
export const turnParams = z.object({
  threadId: z.string(),
  turn: z.object({
    id: z.string(),
    status: z.enum(["completed", "interrupted", "failed", "inProgress"]),
  }),
})
const message = z.object({
  type: z.literal("agentMessage"),
  id: z.string(),
  text: z.string(),
})
const collab = z.object({
  type: z.literal("collabAgentToolCall"),
  id: z.string(),
  tool: z.string(),
  senderThreadId: z.string(),
  receiverThreadIds: z.array(z.string()),
  agentsStates: z.record(
    z.string(),
    z.object({
      status: z.enum([
        "pendingInit",
        "running",
        "interrupted",
        "completed",
        "errored",
        "shutdown",
        "notFound",
      ]),
    })
  ),
})
export const itemParams = identity.extend({
  item: z.union([
    message,
    collab,
    z.object({ type: z.string(), id: z.string() }),
  ]),
})
export const messageItem = message
export const collabItem = collab
