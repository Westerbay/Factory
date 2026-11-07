import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type Feature = components["schemas"]["FeatureView"]
export type FeatureSummary = components["schemas"]["FeatureSummaryView"]
export type FeaturePage = components["schemas"]["FeaturePageView"]
export type FeatureDiff = components["schemas"]["FeatureDiffView"]
export type FeatureAttempt = components["schemas"]["FeatureAttemptView"]
export type FeatureReview = components["schemas"]["FeatureReviewView"]
export type FeatureIntegratedCheck =
  components["schemas"]["FeatureIntegratedCheckView"]
export type FeatureScope = components["schemas"]["FeatureScopeView"]
export type FeatureArchitecture =
  components["schemas"]["FeatureArchitectureView"]
export type FeatureStage = components["schemas"]["FeatureStageView"]
export type FeatureProvider = components["schemas"]["FeatureProvider"]
export type FeatureRun = components["schemas"]["RunView"]
export type CreateFeatureBody = components["schemas"]["CreateFeatureBody"]
export type FeatureRevisionBody = components["schemas"]["FeatureRevisionBody"]
export type FeatureResumeBody = components["schemas"]["FeatureResumeBody"]
export type FeatureSnapshotBody = components["schemas"]["FeatureSnapshotBody"]
export type FeatureTurnBody = components["schemas"]["FeatureTurnBody"]

const id = z.uuid()
const text = z.string()
const stageSchema = z.enum([
  "scope",
  "architecture",
  "execution",
  "review",
  "result",
  "draft",
])
const roleSchema = z.enum([
  "coordinator",
  "architecture",
  "implementation",
  "complexImplementation",
  "visual",
  "triage",
  "generalReview",
  "architectureReview",
  "webReview",
  "securityReview",
  "correction",
])
const summarySchema = z.object({
  id,
  projectId: id,
  conversationId: id,
  provider: text,
  title: text,
  stage: stageSchema,
  createdAt: text,
})
const revisionSchema = z.object({
  id,
  digest: text,
  approvedAt: text.nullish(),
  canApprove: z.boolean(),
})
const scopeSchema = revisionSchema.extend({ brief: text })
const architectureSchema = revisionSchema.extend({
  summary: text,
  diagrams: z.array(z.object({ title: text, source: text })),
})
const attemptSchema = z.object({
  id,
  title: text,
  role: roleSchema,
  assignedModel: text,
  observedModel: text.nullish(),
  effort: text.nullish(),
  status: z.enum([
    "starting",
    "running",
    "validated",
    "failed",
    "interrupted",
    "uncertain",
  ]),
  activity: text.nullish(),
  checkpoint: text.nullish(),
  retryOf: id.nullish(),
  canResume: z.boolean(),
})
const reviewSchema = z.object({
  snapshotId: id,
  baseOid: text,
  headOid: text,
  patchDigest: text,
  ready: z.boolean(),
  canExtend: z.boolean(),
  findings: z.array(
    z.object({ role: roleSchema, summary: text, blocking: z.boolean() })
  ),
  checks: z.array(z.object({ label: text, passed: z.boolean() })),
  integratedChecks: z
    .array(
      z.object({
        command: text,
        status: z.enum(["preparing", "running", "recoveryNeeded", "finished"]),
        passed: z.boolean().nullish(),
      })
    )
    .default([]),
})
const deliverySchema = z.object({
  requestId: id,
  snapshotId: id,
  status: z.enum([
    "claimed",
    "uncertain",
    "failed",
    "published",
    "publishedStale",
  ]),
  reason: text.nullish(),
  draftUrl: z.url({ protocol: /^https$/ }).nullish(),
})
const featureSchema = z.object({
  feature: summarySchema,
  canContinue: z.boolean(),
  scope: scopeSchema.nullish(),
  architecture: architectureSchema.nullish(),
  attempts: z.array(attemptSchema),
  review: reviewSchema.nullish(),
  delivery: deliverySchema.nullish(),
})
const pageSchema = z.object({
  items: z.array(summarySchema),
  nextCursor: id.nullish(),
})
const diffSchema = z.object({
  baseOid: text,
  headOid: text,
  patchDigest: text,
  patch: text,
  changedPaths: z.array(text),
})
const runSchema = z.object({
  id,
  conversationId: id,
  operationId: id,
  model: text,
  effort: text.nullish(),
  status: z.enum(["active", "completed", "failed", "interrupted", "uncertain"]),
})
const errorCodeSchema = z.enum([
  "session-required",
  "forbidden",
  "rate-limited",
  "invalid-input",
  "feature-not-found",
  "feature-conflict",
  "feature-execution-uncertain",
  "feature-review-blocked",
  "feature-stale-snapshot",
  "unavailable",
])
const errorSchema = z.object({ code: errorCodeSchema })
export type FeatureErrorCode = z.infer<typeof errorCodeSchema>

export class FeatureFailure extends ApiFailure {
  constructor(
    status: number,
    public readonly code: FeatureErrorCode
  ) {
    super(status)
  }
}

function checked<T>(result: {
  data?: T
  error?: unknown
  response: Response
}): T {
  if (!result.response.ok) {
    const error = errorSchema.safeParse(result.error)
    throw new FeatureFailure(
      result.response.status,
      error.success ? error.data.code : "unavailable"
    )
  }
  if (result.data === undefined) throw new Error("Missing response")
  return result.data
}

const csrf = (token: string) => ({ "X-CSRF-Token": token })

export async function list(
  query: { projectId?: string; before?: string },
  signal: AbortSignal
): Promise<FeaturePage> {
  return pageSchema.parse(
    checked(
      await apiClient.GET("/api/features", {
        params: { query },
        signal,
      })
    )
  )
}

export async function get(
  featureId: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.GET("/api/features/{id}", {
        params: { path: { id: featureId } },
        signal,
      })
    )
  )
}

export async function create(
  body: CreateFeatureBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features", {
        params: { header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function start(
  featureId: string,
  body: FeatureTurnBody,
  token: string,
  signal: AbortSignal
): Promise<FeatureRun> {
  return runSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/runs", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function approve(
  featureId: string,
  body: FeatureRevisionBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/approve", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function reopen(
  featureId: string,
  body: FeatureRevisionBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/reopen", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function resume(
  featureId: string,
  body: FeatureResumeBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/resume", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function review(
  featureId: string,
  body: FeatureSnapshotBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/review", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function draft(
  featureId: string,
  body: FeatureSnapshotBody,
  token: string,
  signal: AbortSignal
): Promise<Feature> {
  return featureSchema.parse(
    checked(
      await apiClient.POST("/api/features/{id}/draft", {
        params: { path: { id: featureId }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}

export async function diff(
  featureId: string,
  signal: AbortSignal
): Promise<FeatureDiff> {
  return diffSchema.parse(
    checked(
      await apiClient.GET("/api/features/{id}/diff", {
        params: { path: { id: featureId } },
        signal,
      })
    )
  )
}
