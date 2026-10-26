import { z } from "zod"

const id = z.string().regex(/^[A-Za-z0-9_-]{1,128}$/)
const text = z
  .string()
  .min(1)
  .max(65_536)
  .refine((value) => value.trim().length > 0, "Text cannot be blank")
const timestamp = z.iso.datetime()
export const runStatusSchema = z.enum([
  "created",
  "running",
  "waiting-for-user",
  "cancelling",
  "completed",
  "failed",
  "interrupted",
])
export const agentStatusSchema = z.enum([
  "created",
  "running",
  "waiting-for-user",
  "completed",
  "failed",
  "interrupted",
])
export const requestStatusSchema = z.enum([
  "pending",
  "submitting",
  "resolved",
  "invalidated",
])
export const skillReferenceSchema = z.strictObject({
  name: text,
  path: z.string().min(1).max(4096),
})
export const continuationReferenceSchema = z.strictObject({
  runId: id,
  agentId: id,
})
export const agentExecutionSchema = z.strictObject({
  id,
  parentId: id.optional(),
  status: agentStatusSchema,
})
export const questionAnswerSchema = z.union([
  z.strictObject({ optionId: id }),
  z.strictObject({ text }),
])
export const permissionDecisionSchema = z.enum(["allow", "deny"])
export const humanQuestionSchema = z
  .strictObject({
    id,
    agentId: id,
    prompt: text,
    options: z.array(z.strictObject({ id, label: text })).max(100),
    allowFreeText: z.boolean(),
    blocking: z.boolean(),
    status: requestStatusSchema,
  })
  .refine(
    (question) => question.allowFreeText || question.options.length > 0,
    "Question needs a response mode"
  )
  .refine(
    (question) =>
      new Set(question.options.map((option) => option.id)).size ===
      question.options.length,
    "Duplicate option IDs"
  )
export const permissionRequestSchema = z.strictObject({
  id,
  agentId: id,
  description: text,
  status: requestStatusSchema,
})
export const completedMessageSchema = z.strictObject({
  id,
  agentId: id,
  text: z.string().max(1_048_576),
})
export const runSchema = z.strictObject({
  id,
  workspacePath: z.string().min(1).max(4096),
  providerId: id,
  skill: skillReferenceSchema.optional(),
  task: text,
  continuationOf: continuationReferenceSchema.optional(),
  rootAgentId: id,
  status: runStatusSchema,
  version: z.number().int().positive(),
  createdAt: timestamp,
  updatedAt: timestamp,
  reason: text.optional(),
  agents: z.array(agentExecutionSchema),
  questions: z.array(humanQuestionSchema),
  permissions: z.array(permissionRequestSchema),
  messages: z.array(completedMessageSchema),
})
const identity = { id, runId: id, at: timestamp }
export const runEventSchema = z.discriminatedUnion("type", [
  z.strictObject({ ...identity, type: z.literal("run-created") }),
  z.strictObject({ ...identity, type: z.literal("run-started") }),
  z.strictObject({
    ...identity,
    type: z.literal("agent-observed"),
    agentId: id,
    parentId: id,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("agent-finished"),
    agentId: id,
    outcome: z.enum(["completed", "failed", "interrupted"]),
  }),
  z.strictObject({
    ...identity,
    type: z.literal("message-completed"),
    message: completedMessageSchema,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("question-opened"),
    question: humanQuestionSchema,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("question-submitting"),
    questionId: id,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("question-resolved"),
    questionId: id,
    answer: questionAnswerSchema,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("permission-requested"),
    request: permissionRequestSchema,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("permission-submitting"),
    requestId: id,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("permission-resolved"),
    requestId: id,
    decision: permissionDecisionSchema,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("continuation-requested"),
    agentId: id,
    text,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("run-cancelling"),
    reason: text,
  }),
  z.strictObject({ ...identity, type: z.literal("run-completed") }),
  z.strictObject({ ...identity, type: z.literal("run-failed"), reason: text }),
  z.strictObject({
    ...identity,
    type: z.literal("run-interrupted"),
    reason: text,
  }),
  z.strictObject({
    ...identity,
    type: z.literal("provider-disconnected"),
    reason: text,
  }),
])
const pagination = {
  offset: z.number().int().nonnegative(),
  limit: z.number().int().min(1).max(200),
}
/** Workspace/skill IDs are host-issued selections; renderer cannot submit filesystem roots. */
export const desktopCommandSchema = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("probe") }),
  z.strictObject({ type: z.literal("list-skills"), workspaceId: id }),
  z.strictObject({
    type: z.literal("start-run"),
    workspaceId: id,
    providerId: id,
    task: text,
    skillId: id.optional(),
  }),
  z.strictObject({
    type: z.literal("continue-run"),
    runId: id,
    agentId: id,
    text,
  }),
  z.strictObject({
    type: z.literal("answer-question"),
    runId: id,
    agentId: id,
    questionId: id,
    answer: questionAnswerSchema,
  }),
  z.strictObject({
    type: z.literal("answer-permission"),
    runId: id,
    agentId: id,
    requestId: id,
    decision: permissionDecisionSchema,
  }),
  z.strictObject({ type: z.literal("cancel-run"), runId: id }),
  z.strictObject({ type: z.literal("get-run"), runId: id }),
  z.strictObject({ type: z.literal("list-runs"), ...pagination }),
  z.strictObject({
    type: z.literal("history"),
    runId: id,
    afterSequence: z.number().int().nonnegative(),
    limit: pagination.limit,
  }),
])
export const runtimeProbeSchema = z.strictObject({
  providerId: id,
  available: z.boolean(),
  reason: text.optional(),
  capabilities: z.strictObject({
    nativeDelegation: z.boolean(),
    structuredQuestions: z.boolean(),
    resume: z.boolean(),
  }),
})
export const errorCodeSchema = z.enum([
  "active-run",
  "not-found",
  "ownership",
  "invalid-transition",
  "stale-request",
  "invalid-answer",
  "unsupported",
  "persistence",
  "runtime",
  "unavailable",
])
export const desktopEventSchema = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("run-state"), run: runSchema }),
  z.strictObject({
    type: z.literal("run-event"),
    durable: z.literal(true),
    sequence: z.number().int().positive(),
    event: runEventSchema,
  }),
  z.strictObject({
    type: z.literal("message-delta"),
    durable: z.literal(false),
    ...identity,
    agentId: id,
    messageId: id,
    delta: z.string().max(65_536),
  }),
  z.strictObject({
    type: z.literal("diagnostic"),
    code: errorCodeSchema,
    message: text,
    runId: id.optional(),
  }),
])
export const desktopResultSchema = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("run"), run: runSchema }),
  z.strictObject({
    type: z.literal("runs"),
    runs: z.array(runSchema),
    nextOffset: z.number().int().nonnegative().optional(),
  }),
  z.strictObject({
    type: z.literal("history"),
    events: z.array(runEventSchema),
    nextSequence: z.number().int().positive().optional(),
  }),
  z.strictObject({ type: z.literal("probe"), probe: runtimeProbeSchema }),
  z.strictObject({
    type: z.literal("skills"),
    skills: z.array(z.strictObject({ id, reference: skillReferenceSchema })),
  }),
  z.strictObject({ type: z.literal("acknowledged") }),
  z.strictObject({
    type: z.literal("error"),
    code: errorCodeSchema,
    message: text,
  }),
])
export type DesktopCommand = z.infer<typeof desktopCommandSchema>
export type DesktopEvent = z.infer<typeof desktopEventSchema>
export type DesktopResult = z.infer<typeof desktopResultSchema>
export type RunProjection = z.infer<typeof runSchema>
export type MaterialEvent = z.infer<typeof runEventSchema>
