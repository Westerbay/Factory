import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

vi.mock("../../src/lib/theme/theme", () => ({ useTheme: () => "dark" }))

import type {
  Feature,
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
  },
}
function render(
  value: Feature,
  options: { busy?: boolean; diffError?: boolean; diff?: FeatureDiff } = {}
) {
  return renderToStaticMarkup(
    <FeaturePanels
      feature={value}
      diff={options.diff ?? diff}
      diffError={options.diffError ?? false}
      diffLoading={false}
      busy={options.busy ?? false}
      onApprove={() => {}}
      onReopen={() => {}}
      onResume={() => {}}
      onReview={() => {}}
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
    expect(html).not.toContain('title="reviewed-head"')
    expect(html).not.toContain("Ready for your review")
    expect(html).not.toContain("Review this diff")
    expect(html).not.toContain("Publish the reviewed draft")
    expect(html).toContain('href="https://github.com/owner/repo/pull/12"')
    expect(html).toContain("+ remote change")
    expect(html).not.toContain("Prior snapshot finding")
    expect(html).not.toContain("Prior snapshot typecheck")
    expect(html).not.toContain('data-passed="true"')
  })

  it("shows review findings and checks only for the displayed exact reviewed patch", () => {
    const bound = render(reviewedFeature, { diff: reviewedDiff })
    expect(bound).toContain("Prior snapshot finding")
    expect(bound).toContain("Prior snapshot typecheck")
    expect(bound).toContain('data-passed="true"')
    expect(bound).toContain("Ready for your review")
    for (const key of ["baseOid", "headOid", "patchDigest"] as const) {
      const changed = render(reviewedFeature, {
        diff: { ...reviewedDiff, [key]: "changed" },
      })
      expect(changed).not.toContain("Prior snapshot finding")
      expect(changed).not.toContain("Prior snapshot typecheck")
      expect(changed).not.toContain("Ready for your review")
      if (key !== "patchDigest") expect(changed).toContain('title="changed"')
      expect(changed).toContain("+ remote change")
    }
    const unavailable = render(reviewedFeature, {
      diff: reviewedDiff,
      diffError: true,
    })
    expect(unavailable).not.toContain("Prior snapshot finding")
    expect(unavailable).not.toContain("Prior snapshot typecheck")
    expect(unavailable).not.toContain("Ready for your review")
    const stale = render(
      {
        ...reviewedFeature,
        delivery: { ...receipt, status: "publishedStale" },
      },
      { diff: reviewedDiff }
    )
    expect(stale).not.toContain("Prior snapshot finding")
    expect(stale).not.toContain("Prior snapshot typecheck")
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

  it("offers reopening a settled result and only exposes step continuation projected by the server", () => {
    expect(render(feature)).toContain("Revisit product decisions")
    expect(render(feature)).not.toContain("Resume this step")
    expect(render({ ...feature, canContinue: true })).toContain(
      "Resume this step"
    )
  })
})
