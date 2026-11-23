import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

vi.mock("../../src/lib/theme/theme", () => ({ useTheme: () => "dark" }))

import type {
  Feature,
  FeatureAttempt,
  FeatureDiff,
} from "../../src/features/feature-workflows/api"
import { FeaturePanels } from "../../src/features/feature-workflows/feature-panels"

const diff: FeatureDiff = {
  baseOid: "remote-base",
  headOid: "remote-head",
  patchDigest: "remote-digest",
  changedPaths: ["view.tsx"],
  patch: "+ remote change",
}
const feature: Feature = {
  feature: {
    id: "feature",
    conversationId: "conversation",
    projectId: "project",
    provider: "codex",
    title: "Feature",
    createdAt: "2026-11-07",
    stage: "result",
  },
  canContinue: false,
  attempts: [],
  scope: {
    id: "scope",
    digest: "scope",
    brief: "Brief",
    approvedAt: "approved",
    canApprove: false,
  },
  architecture: {
    id: "architecture",
    digest: "architecture",
    summary: "Architecture",
    diagrams: [],
    approvedAt: "approved",
    canApprove: false,
  },
  review: {
    snapshotId: "snapshot",
    baseOid: "reviewed-base",
    headOid: "reviewed-head",
    patchDigest: "reviewed-digest",
    ready: true,
    canExtend: false,
    findings: [],
    checks: [],
    integratedChecks: [],
  },
}
function render(
  value: Feature,
  options: {
    busy?: boolean
    diffError?: boolean
    diff?: FeatureDiff
    viewStage?: Feature["feature"]["stage"]
    mergeResolutionPending?: boolean
    mergeResolutionError?: string
  } = {}
) {
  return renderToStaticMarkup(
    <FeaturePanels
      feature={value}
      diff={options.diff ?? diff}
      diffError={options.diffError ?? false}
      diffLoading={false}
      busy={options.busy ?? false}
      viewStage={options.viewStage}
      onApprove={() => {}}
      onReopen={() => {}}
      onResume={() => {}}
      onReview={() => {}}
      onResolveMergeConflict={() => {}}
      onRefreshMergeResolution={() => {}}
      mergeResolutionPending={options.mergeResolutionPending}
      mergeResolutionError={options.mergeResolutionError}
      onDraft={() => {}}
      onReloadDiff={() => {}}
      onDiscuss={() => {}}
    />
  )
}
const receipt = {
  requestId: "durable-request",
  snapshotId: "snapshot",
  status: "uncertain" as const,
}
const reviewedFeature: Feature = {
  ...feature,
  review: {
    ...feature.review!,
    findings: [
      {
        role: "generalReview",
        summary: "Prior snapshot finding",
        blocking: false,
      },
    ],
    checks: [{ label: "Prior snapshot typecheck", passed: true }],
  },
}
const reviewedDiff: FeatureDiff = {
  ...diff,
  baseOid: reviewedFeature.review!.baseOid,
  headOid: reviewedFeature.review!.headOid,
  patchDigest: reviewedFeature.review!.patchDigest,
}
const conflicted: Feature = {
  ...reviewedFeature,
  feature: { ...reviewedFeature.feature, stage: "review" },
  delivery: {
    ...receipt,
    status: "failed",
    reason:
      "The target branch has changes that conflict with this feature. Resolve the conflicts against the current target branch before opening a draft.",
  },
}
const resolution: FeatureAttempt = {
  id: "resolution",
  title: "Resolve merge conflicts",
  role: "correction",
  assignedModel: "codex",
  status: "running",
  canResume: false,
}

describe("architecture preparation", () => {
  const preparing: Feature = {
    ...feature,
    feature: { ...feature.feature, stage: "architecture" },
    architecture: undefined,
  }

  it("distinguishes a confirmed brief and in-progress generation from an unapproved brief", () => {
    const active = render(preparing)
    expect(active).toContain("Product brief approved")
    expect(active).toContain("Preparing the architecture proposal")
    expect(active).not.toContain("after the product brief is approved")

    const waiting = render(
      {
        ...preparing,
        feature: { ...preparing.feature, stage: "scope" },
        scope: { ...feature.scope!, approvedAt: null },
      },
      { viewStage: "architecture" }
    )
    expect(waiting).toContain("after the product brief is approved")
    expect(waiting).not.toContain("Preparing the architecture proposal")
  })

  it("offers recovery when architecture could not start or was interrupted", () => {
    const notStarted = render({ ...preparing, canContinue: true })
    expect(notStarted).toContain("Architecture preparation has not started")
    expect(notStarted).not.toContain("Preparing the architecture proposal")

    const interrupted = render({
      ...preparing,
      attempts: [
        {
          id: "architecture-attempt",
          title: "Architecture",
          assignedModel: "model",
          role: "architecture",
          status: "interrupted",
          canResume: true,
        },
      ],
    })
    expect(interrupted).toContain("Architecture preparation was interrupted")
    expect(interrupted).not.toContain("Preparing the architecture proposal")
  })
})

describe("feature result actions", () => {
  it("keeps reconciliation available for a changed diff and disables it when commands are unavailable", () => {
    const value = { ...feature, delivery: receipt }
    const button = render(value).match(
      /<button[^>]*>Check the draft request[\s\S]*?<\/button>/
    )?.[0]
    expect(button).toBeDefined()
    expect(button).not.toContain("disabled")
    expect(render(value)).not.toContain("Review this diff")
    expect(render(value, { busy: true })).toMatch(
      /<button[^>]*disabled=""[^>]*>Check the draft request/
    )
  })

  it("shows stale remote receipt and fetched remote identity without Ready or another review/publication action", () => {
    const html = render({
      ...reviewedFeature,
      delivery: {
        ...receipt,
        status: "publishedStale",
        draftUrl: "https://github.com/owner/repo/pull/12",
      },
    })
    expect(html).toContain("Published draft needs review")
    expect(html).toContain('title="remote-base"')
    expect(html).toContain('title="remote-head"')
    expect(html).toContain("<code>reviewed-head</code>")
    expect(html).not.toContain("Ready for your review")
    expect(html).not.toContain("Review this diff")
    expect(html).not.toContain("Publish the reviewed draft")
    expect(html).toContain('href="https://github.com/owner/repo/pull/12"')
    expect(html).toContain("+ remote change")
    expect(html).toContain('aria-label="Previous reviewed snapshot"')
    expect(html).toContain(
      "Findings and checks below apply to this earlier snapshot."
    )
    expect(html).toContain("Prior snapshot finding")
    expect(html).toContain("Prior snapshot typecheck")
    expect(html).toContain("<code>reviewed-digest</code>")
  })

  it("binds findings and checks to their exact prior snapshot when the current diff differs or is unavailable", () => {
    const bound = render(reviewedFeature, { diff: reviewedDiff })
    expect(bound).toContain("Prior snapshot finding")
    expect(bound).toContain("Prior snapshot typecheck")
    expect(bound).toContain('data-passed="true"')
    expect(bound).toContain("Ready for your review")
    expect(bound).toContain('aria-label="Review of the displayed patch"')
    expect(bound).not.toContain('data-previous=""')
    for (const key of ["baseOid", "headOid", "patchDigest"] as const) {
      const changed = render(reviewedFeature, {
        diff: { ...reviewedDiff, [key]: "changed" },
      })
      expect(changed).toContain('aria-label="Previous reviewed snapshot"')
      expect(changed).toContain("<code>reviewed-base</code>")
      expect(changed).toContain("<code>reviewed-head</code>")
      expect(changed).toContain("<code>reviewed-digest</code>")
      expect(changed).toContain("Prior snapshot finding")
      expect(changed).toContain("Prior snapshot typecheck")
      expect(changed).not.toContain("Ready for your review")
      if (key !== "patchDigest") expect(changed).toContain('title="changed"')
      expect(changed).toContain("+ remote change")
    }
    const unavailable = render(reviewedFeature, {
      diff: reviewedDiff,
      diffError: true,
    })
    expect(unavailable).toContain('aria-label="Previous reviewed snapshot"')
    expect(unavailable).toContain("Prior snapshot finding")
    expect(unavailable).toContain("Prior snapshot typecheck")
    expect(unavailable).not.toContain("Ready for your review")
    const stale = render(
      {
        ...reviewedFeature,
        delivery: { ...receipt, status: "publishedStale" },
      },
      { diff: reviewedDiff }
    )
    expect(stale).toContain('aria-label="Previous reviewed snapshot"')
    expect(stale).toContain("Prior snapshot finding")
    expect(stale).toContain("Prior snapshot typecheck")
  })

  it("does not substitute the local review identity when the published diff is unavailable", () => {
    const html = render(
      { ...feature, delivery: { ...receipt, status: "publishedStale" } },
      { diffError: true }
    )
    expect(html).toContain("The published PR diff is unavailable.")
    expect(html).not.toContain('title="reviewed-head"')
    expect(html).not.toContain('title="remote-head"')
  })

  it("does not show Ready for a review invalidated while its old comparison is still available", () => {
    const invalidated = {
      ...reviewedFeature,
      feature: { ...reviewedFeature.feature, stage: "review" as const },
      delivery: {
        ...receipt,
        status: "failed" as const,
        reason: "Factory could not prepare the current comparison.",
      },
    }
    const html = render(invalidated, { diff: reviewedDiff })
    expect(html).not.toContain("Ready for your review")
    expect(html).not.toContain("Publish the reviewed draft")
    expect(html).toContain("Factory could not prepare the current comparison.")
    expect(html).toContain("Refresh and review the diff")
    expect(html).toContain('aria-label="Previous reviewed snapshot"')
    expect(html).toContain("Prior snapshot typecheck")
  })

  it("explains a target branch conflict without offering to publish the previous patch", () => {
    const html = render(conflicted, { diff: reviewedDiff })
    expect(html).toContain(
      "Resolve the conflicts against the current target branch"
    )
    expect(html).not.toContain("Publish the reviewed draft")
    expect(html).not.toContain("Attempt resolution")
    expect(html).toContain('aria-label="Previous reviewed snapshot"')
  })

  it("offers one eligible action and keeps it guarded while the request is pending", () => {
    const eligible = { ...conflicted, canResolveMergeConflict: true }
    const html = render(eligible, { diff: reviewedDiff })
    expect(html.match(/Attempt resolution/g)).toHaveLength(1)
    expect(html).toMatch(/<button type="button"[^>]*>Attempt resolution/)
    expect(html).not.toContain("Publish the reviewed draft")

    const pending = render(eligible, {
      diff: reviewedDiff,
      mergeResolutionPending: true,
    })
    expect(pending).toMatch(/<button disabled=""[^>]*>Attempt resolution/)
    expect(pending).toContain(
      'role="status" aria-atomic="true"><p>Preparing conflict resolution'
    )
  })

  it("shows the named attempt while retaining the previous review on failure", () => {
    const value = {
      ...conflicted,
      mergeResolutionAttemptId: resolution.id,
      attempts: [
        resolution,
        { ...resolution, id: "unrelated", status: "failed" as const },
      ],
    }
    const running = render(value, { diff: reviewedDiff })
    expect(running).toContain("An agent is resolving the conflicts")
    expect(running).not.toContain("Attempt resolution")
    expect(running).not.toContain("Publish the reviewed draft")
    const failed = render(
      {
        ...value,
        canResolveMergeConflict: true,
        attempts: [
          { ...resolution, status: "failed", activity: "The target changed." },
        ],
      },
      { diff: reviewedDiff }
    )
    expect(failed).toContain("Try again or resolve the conflicts manually")
    expect(failed).toContain("The target changed.")
    expect(failed).toContain('aria-label="Previous reviewed snapshot"')
    expect(failed).toContain("Prior snapshot typecheck")
    expect(failed).toContain("Attempt resolution")
    expect(failed).not.toContain("Publish the reviewed draft")
  })

  it("offers recovery for an uncertain resolution, and a status check for a rejected request", () => {
    const uncertain = render(
      {
        ...conflicted,
        mergeResolutionAttemptId: resolution.id,
        attempts: [{ ...resolution, status: "uncertain", canResume: true }],
      },
      { diff: reviewedDiff }
    )
    expect(uncertain).toContain("Recover the resolution attempt")
    expect(uncertain).not.toContain("Attempt resolution")

    const rejected = render(conflicted, {
      diff: reviewedDiff,
      mergeResolutionError: "Resolve these conflicts manually.",
    })
    expect(rejected).toContain(
      'role="status" aria-atomic="true"><p>Resolve these conflicts manually.'
    )
    expect(rejected).toContain("Check feature status")
  })

  it("waits for review of the updated diff before offering explicit publication", () => {
    const updated: Feature = {
      ...conflicted,
      review: {
        ...feature.review!,
        snapshotId: "updated",
        baseOid: diff.baseOid,
        headOid: diff.headOid,
        patchDigest: diff.patchDigest,
        ready: false,
      },
      attempts: [{ ...resolution, status: "validated" }],
    }
    const reviewing = render(updated, { diff })
    expect(reviewing).toContain("Reviewing the updated diff")
    expect(reviewing).toContain("Previous publication attempt")
    expect(reviewing).not.toContain("Attempt resolution")
    expect(reviewing).not.toContain("Publish the reviewed draft")
    const ready = render(
      {
        ...updated,
        feature: { ...updated.feature, stage: "result" },
        review: { ...updated.review!, ready: true },
      },
      { diff }
    )
    expect(ready).toContain("Ready for your review")
    expect(ready).toMatch(
      /<button type="button"[^>]*>Publish the reviewed draft/
    )
  })

  it("offers reopening a settled result and only exposes step continuation projected by the server", () => {
    expect(render(feature)).toContain("Revisit product decisions")
    expect(render(feature)).not.toContain("Resume this step")
    expect(render({ ...feature, canContinue: true })).toContain(
      "Resume this step"
    )
  })

  it("keeps the exact prior blocking evidence readable during correction when comparison is unavailable", () => {
    const correcting: Feature = {
      ...reviewedFeature,
      feature: { ...reviewedFeature.feature, stage: "execution" },
      review: {
        ...reviewedFeature.review!,
        ready: false,
        findings: [
          {
            role: "securityReview",
            summary: "The prior route is missing its access check",
            blocking: true,
          },
        ],
        checks: [{ label: "Prior authorization test", passed: false }],
      },
    }
    const html = render(correcting, { diffError: true, viewStage: "result" })
    expect(html).toContain('aria-label="Previous reviewed snapshot"')
    expect(html).toContain("<code>reviewed-base</code>")
    expect(html).toContain("<code>reviewed-head</code>")
    expect(html).toContain("<code>reviewed-digest</code>")
    expect(html).toContain("The prior route is missing its access check")
    expect(html).toContain("Prior authorization test")
    expect(html).not.toContain("Ready for your review")
  })

  it.each([
    ["preparing", "Preparation pending", "Workspace preparation is pending"],
    [
      "running",
      "Verification in progress",
      "Its result has not been confirmed yet",
    ],
    [
      "recoveryNeeded",
      "Recovery needs confirmation",
      "Factory is waiting to confirm that the last verification stopped",
    ],
  ] as const)(
    "shows the projected %s command and reason without inferring recovery authority",
    (status, label, reason) => {
      const value: Feature = {
        ...feature,
        feature: { ...feature.feature, stage: "review" },
        review: {
          ...feature.review!,
          ready: false,
          integratedChecks: [
            { command: "pnpm test --filter integration", status },
          ],
        },
      }
      const html = render(value, { diff: reviewedDiff })
      expect(html).toContain("pnpm test --filter integration")
      expect(html).toContain(`role="status">${label}</span>`)
      expect(html).toContain(reason)
      expect(html).not.toContain("Resume this step</button>")
      expect(html).not.toContain("Ready for your review")
      expect(html).not.toContain("No findings were reported for this snapshot.")
      expect(html).not.toContain("No verification result has been recorded.")
      const unavailable = render(value, { diffError: true })
      expect(unavailable).toContain('aria-label="Review snapshot"')
      expect(unavailable).not.toContain("Previous reviewed snapshot")
      expect(unavailable).toContain("<code>reviewed-digest</code>")
      expect(
        render({ ...value, canContinue: true }, { diff: reviewedDiff })
      ).toContain("Resume this step</button>")
    }
  )

  it("shows the server-confirmed failed settlement reason without offering another reconciliation", () => {
    const html = render(
      {
        ...feature,
        delivery: {
          ...receipt,
          status: "failed",
          reason:
            "No draft exists for this verified request; publication was released.",
        },
      },
      { diff: reviewedDiff }
    )
    expect(html).toContain(
      "No draft exists for this verified request; publication was released."
    )
    expect(html).not.toContain("Check the draft request")
  })
})
