import { describe, expect, it } from "vitest"

import type {
  Feature,
  FeatureAttempt,
  FeatureDiff,
} from "../../src/features/feature-workflows/api"
import {
  canPublish,
  canReconcileDraft,
  createOperationRegistry,
  hasActiveAttempts,
  hasUncertainAttempts,
  matchesReviewedDiff,
} from "../../src/features/feature-workflows/state"

const snapshotId = "65f0bb33-a642-451e-b599-6bd8e9c970f9"
const diff: FeatureDiff = {
  baseOid: "base",
  headOid: "head",
  patchDigest: "digest",
  patch: "+ change",
  changedPaths: ["src/view.tsx"],
}
const feature: Feature = {
  canContinue: false,
  feature: {
    id: "d8863923-1c56-4c20-a8b2-8acfe2fccb177",
    projectId: "f3912a9a-051d-415c-94b5-e6c32b3606b6",
    conversationId: "6e603cdb-6408-4853-9a0b-ab29addcaa97",
    provider: "codex",
    title: "Feature",
    stage: "result",
    createdAt: "2026-11-07T14:00:00Z",
  },
  attempts: [],
  review: {
    snapshotId,
    baseOid: diff.baseOid,
    headOid: diff.headOid,
    patchDigest: diff.patchDigest,
    ready: true,
    checks: [{ label: "Typecheck", passed: true }],
    findings: [],
    canExtend: false,
  },
}
const attempt: FeatureAttempt = {
  id: "264c0824-e178-43f4-a111-e2240077c951",
  title: "Implementation",
  role: "implementation",
  assignedModel: "Terra",
  status: "running",
  canResume: false,
}

describe("reviewed publication guard", () => {
  it("requires all diff identity fields and confirmed review evidence", () => {
    expect(canPublish(feature, diff)).toBe(true)
    expect(canPublish(feature, undefined)).toBe(false)
    for (const key of ["baseOid", "headOid", "patchDigest"] as const) {
      const changed = { ...diff, [key]: "changed" }
      expect(matchesReviewedDiff(feature, changed)).toBe(false)
      expect(canPublish(feature, changed)).toBe(false)
    }
    expect(canPublish({ ...feature, review: null }, diff)).toBe(false)
    expect(
      canPublish(
        { ...feature, review: { ...feature.review!, ready: false } },
        diff
      )
    ).toBe(false)
    expect(
      canPublish(
        {
          ...feature,
          review: {
            ...feature.review!,
            checks: [{ label: "Build", passed: false }],
          },
        },
        diff
      )
    ).toBe(false)
    expect(
      canPublish(
        {
          ...feature,
          review: {
            ...feature.review!,
            findings: [
              { role: "securityReview", summary: "Must fix", blocking: true },
            ],
          },
        },
        diff
      )
    ).toBe(false)
  })

  it("keeps in-flight and uncertain attempts blocking without treating a prior failed attempt as active", () => {
    for (const status of ["starting", "running", "uncertain"] as const) {
      expect(
        canPublish({ ...feature, attempts: [{ ...attempt, status }] }, diff)
      ).toBe(false)
    }
    expect(hasActiveAttempts({ ...feature, attempts: [attempt] })).toBe(true)
    expect(
      hasUncertainAttempts({
        ...feature,
        attempts: [{ ...attempt, status: "uncertain" }],
      })
    ).toBe(true)
    const completed = {
      ...feature,
      attempts: [
        { ...attempt, status: "failed" as const },
        { ...attempt, status: "validated" as const },
      ],
    }
    expect(hasActiveAttempts(completed)).toBe(false)
    expect(hasUncertainAttempts(completed)).toBe(false)
    expect(canPublish(completed, diff)).toBe(true)
  })

  it("separates fresh publication from pending or completed delivery receipts", () => {
    const receipt = {
      requestId: "62f21847-4f2c-4314-be5b-747e3a8ba882",
      snapshotId,
      status: "uncertain" as const,
    }
    for (const status of ["claimed", "uncertain"] as const) {
      expect(
        canPublish({ ...feature, delivery: { ...receipt, status } }, diff)
      ).toBe(false)
    }
    expect(
      canPublish(
        {
          ...feature,
          delivery: { ...receipt, snapshotId: "another-snapshot" },
        },
        diff
      )
    ).toBe(false)
    expect(
      canPublish(
        { ...feature, delivery: { ...receipt, status: "failed" } },
        diff
      )
    ).toBe(true)
    for (const status of ["published", "publishedStale"] as const) {
      expect(
        canPublish({ ...feature, delivery: { ...receipt, status } }, diff)
      ).toBe(false)
    }
    expect(
      canPublish(
        { ...feature, feature: { ...feature.feature, stage: "review" } },
        diff
      )
    ).toBe(false)
  })

  it("reconciles a durable pending request even when the local diff or review changed", () => {
    const receipt = {
      requestId: "62f21847-4f2c-4314-be5b-747e3a8ba882",
      snapshotId,
      status: "claimed" as const,
    }
    for (const status of ["claimed", "uncertain"] as const) {
      const pending = { ...feature, delivery: { ...receipt, status } }
      expect(canReconcileDraft(pending)).toBe(true)
      for (const key of ["baseOid", "headOid", "patchDigest"] as const) {
        expect(canPublish(pending, { ...diff, [key]: "changed" })).toBe(false)
        expect(canReconcileDraft(pending)).toBe(true)
      }
      expect(
        canReconcileDraft({
          ...pending,
          feature: { ...feature.feature, stage: "review" },
          review: null,
          attempts: [{ ...attempt, status: "uncertain" }],
        })
      ).toBe(true)
    }
    expect(canReconcileDraft(feature)).toBe(false)
    for (const status of ["failed", "published", "publishedStale"] as const) {
      expect(
        canReconcileDraft({ ...feature, delivery: { ...receipt, status } })
      ).toBe(false)
    }
    expect(
      canReconcileDraft({ ...feature, delivery: { ...receipt, requestId: "" } })
    ).toBe(false)
    expect(
      canReconcileDraft({
        ...feature,
        delivery: { ...receipt, snapshotId: "" },
      })
    ).toBe(false)
  })
})

describe("session operation registry", () => {
  it("retains an uncertain operation until its exact response is acknowledged", () => {
    const registry = createOperationRegistry()
    const id = registry.get("session-1", "approve:feature:revision:digest")
    expect(registry.get("session-1", "approve:feature:revision:digest")).toBe(
      id
    )
    expect(
      registry.get("session-1", "approve:feature:other-revision:digest")
    ).not.toBe(id)
    registry.complete(
      "session-1",
      "approve:feature:revision:digest",
      "unrelated-response"
    )
    expect(registry.get("session-1", "approve:feature:revision:digest")).toBe(
      id
    )
    registry.complete("session-1", "approve:feature:revision:digest", id)
    expect(
      registry.get("session-1", "approve:feature:revision:digest")
    ).not.toBe(id)
  })

  it("discards another security scope and ignores late responses from that scope", () => {
    const registry = createOperationRegistry()
    const previous = registry.get("session-1", "draft:feature:snapshot")
    const current = registry.get("session-2", "draft:feature:snapshot")
    expect(current).not.toBe(previous)
    registry.complete("session-1", "draft:feature:snapshot", previous)
    expect(registry.get("session-2", "draft:feature:snapshot")).toBe(current)
    registry.clear()
    expect(registry.get("session-2", "draft:feature:snapshot")).not.toBe(
      current
    )
  })
})
