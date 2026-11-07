import type { Feature, FeatureDiff } from "./api"

export function hasActiveAttempts(feature: Feature): boolean {
  return feature.attempts.some(
    (attempt) => attempt.status === "starting" || attempt.status === "running"
  )
}

export function hasUncertainAttempts(feature: Feature): boolean {
  return feature.attempts.some((attempt) => attempt.status === "uncertain")
}

/** A review verdict applies only to the complete fetched diff identity. */
export function matchesReviewedDiff(
  feature: Feature,
  diff: FeatureDiff | null | undefined
): boolean {
  const review = feature.review
  return Boolean(
    review &&
    diff &&
    review.baseOid === diff.baseOid &&
    review.headOid === diff.headOid &&
    review.patchDigest === diff.patchDigest
  )
}

/** Presentation guard; the server rechecks the snapshot before either external write. */
export function canPublish(
  feature: Feature,
  diff: FeatureDiff | null | undefined
): boolean {
  const review = feature.review
  const delivery = feature.delivery
  return Boolean(
    feature.feature.stage === "result" &&
    review?.ready &&
    review.checks.every((check) => check.passed) &&
    !review.findings.some((finding) => finding.blocking) &&
    matchesReviewedDiff(feature, diff) &&
    !hasActiveAttempts(feature) &&
    !hasUncertainAttempts(feature) &&
    (!delivery || delivery.status === "failed")
  )
}

/** Reconcile the durable request even if the current local diff has moved. */
export function canReconcileDraft(feature: Feature): boolean {
  const delivery = feature.delivery
  return Boolean(
    delivery?.requestId &&
    delivery.snapshotId &&
    (delivery.status === "claimed" || delivery.status === "uncertain")
  )
}

/**
 * Keep uncertain commands stable across view remounts within one authenticated tree.
 * The workspace owns this registry and clears it on unmount or session loss.
 * Keys identify exact command payloads; no messages or credentials are persisted.
 */
export function createOperationRegistry() {
  let currentScope: string | undefined
  const pending = new Map<string, string>()
  return {
    get(scope: string, key: string): string {
      if (scope !== currentScope) {
        pending.clear()
        currentScope = scope
      }
      const existing = pending.get(key)
      if (existing) return existing
      const operationId = crypto.randomUUID()
      pending.set(key, operationId)
      return operationId
    },
    complete(scope: string, key: string, operationId: string) {
      if (scope === currentScope && pending.get(key) === operationId)
        pending.delete(key)
    },
    clear() {
      pending.clear()
      currentScope = undefined
    },
  }
}
