import { afterEach, describe, expect, it, vi } from "vitest"

import { apiClient, ApiFailure } from "../../src/lib/api/client"
import * as api from "../../src/features/feature-workflows/api"

const featureId = "7c31d10a-8b79-4adf-a3f7-88fe2c4868dd"
const conversationId = "b37ff425-0f94-43b5-b082-835b7ec81056"
const projectId = "8bc4b4c1-f8cd-4322-973b-1126b9c93876"
const operationId = "e3507c3a-40d5-4af7-a54e-a9d468289116"
const revisionId = "d9b35396-1fbf-4d1d-841c-d4a8d334523a"
const feature: api.Feature = {
  canContinue: false,
  feature: {
    id: featureId,
    conversationId,
    projectId,
    provider: "codex",
    title: "New feature",
    stage: "scope",
    createdAt: "2026-11-07T14:00:00Z",
  },
  scope: null,
  architecture: null,
  attempts: [],
  review: null,
  delivery: null,
}
const ok = (data: unknown) =>
  ({
    data,
    response: new Response(null, { status: 200 }),
  }) as never

afterEach(() => vi.restoreAllMocks())

describe("feature API", () => {
  it("validates paginated project-scoped lists instead of accepting unknown workflow stages", async () => {
    const get = vi
      .spyOn(apiClient, "GET")
      .mockResolvedValueOnce(
        ok({ items: [feature.feature], nextCursor: featureId })
      )
      .mockResolvedValueOnce(
        ok({ items: [{ ...feature.feature, stage: "approved" }] })
      )
    const signal = new AbortController().signal
    expect(await api.list({ projectId, before: featureId }, signal)).toEqual({
      items: [feature.feature],
      nextCursor: featureId,
    })
    expect(get).toHaveBeenCalledWith("/api/features", {
      params: { query: { projectId, before: featureId } },
      signal,
    })
    await expect(api.list({}, signal)).rejects.toThrow()
  })

  it("creates with the caller's operation, account, locale and CSRF while preserving the conversation identity", async () => {
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce(ok(feature))
    const signal = new AbortController().signal
    const body: api.CreateFeatureBody = {
      operationId,
      projectId,
      provider: "codex",
      locale: "fr",
      title: "Nouvelle feature",
    }
    expect(
      (await api.create(body, "csrf", signal)).feature.conversationId
    ).toBe(conversationId)
    expect(post).toHaveBeenCalledWith("/api/features", {
      params: { header: { "X-CSRF-Token": "csrf" } },
      body,
      signal,
    })
  })

  it("binds revision, recovery and publication commands to their exact supplied identity", async () => {
    const post = vi.spyOn(apiClient, "POST").mockResolvedValue(ok(feature))
    const signal = new AbortController().signal
    const revision = {
      operationId,
      revisionId,
      digest: "current-revision-digest",
    }
    const snapshot = { operationId, snapshotId: revisionId }
    const attempt = { operationId, attemptId: revisionId }
    await api.approve(featureId, revision, "csrf", signal)
    await api.reopen(featureId, revision, "csrf", signal)
    await api.resume(featureId, attempt, "csrf", signal)
    await api.review(featureId, snapshot, "csrf", signal)
    await api.draft(featureId, snapshot, "csrf", signal)
    for (const [index, [path, body]] of [
      ["approve", revision],
      ["reopen", revision],
      ["resume", attempt],
      ["review", snapshot],
      ["draft", snapshot],
    ].entries()) {
      expect(post).toHaveBeenNthCalledWith(
        index + 1,
        `/api/features/{id}/${path}`,
        {
          params: {
            path: { id: featureId },
            header: { "X-CSRF-Token": "csrf" },
          },
          body,
          signal,
        }
      )
    }
  })

  it("sends discussion through the workflow endpoint without choosing a client model", async () => {
    const run: api.FeatureRun = {
      id: revisionId,
      conversationId,
      operationId,
      model: "factory:codex:resolved-profile",
      effort: "high",
      status: "active",
    }
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce(ok(run))
    const signal = new AbortController().signal
    const body = { operationId, input: "Clarify the expected behavior." }
    expect(await api.start(featureId, body, "csrf", signal)).toEqual(run)
    expect(post).toHaveBeenCalledWith("/api/features/{id}/runs", {
      params: { path: { id: featureId }, header: { "X-CSRF-Token": "csrf" } },
      body,
      signal,
    })
  })

  it("continues an idle workflow without supplying an attempt to retry", async () => {
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce(ok(feature))
    const signal = new AbortController().signal
    const body: api.FeatureResumeBody = { operationId }
    expect(await api.resume(featureId, body, "csrf", signal)).toEqual(feature)
    expect(post).toHaveBeenCalledWith("/api/features/{id}/resume", {
      params: {
        path: { id: featureId },
        header: { "X-CSRF-Token": "csrf" },
      },
      body: { operationId },
      signal,
    })
  })

  it("preserves explicit continuation authority and rejects a missing flag", async () => {
    vi.spyOn(apiClient, "GET")
      .mockResolvedValueOnce(ok({ ...feature, canContinue: true }))
      .mockResolvedValueOnce(ok(feature))
      .mockResolvedValueOnce(ok({ ...feature, canContinue: undefined }))
    const signal = new AbortController().signal
    expect((await api.get(featureId, signal)).canContinue).toBe(true)
    expect((await api.get(featureId, signal)).canContinue).toBe(false)
    await expect(api.get(featureId, signal)).rejects.toThrow()
  })

  it("preserves bounded feature errors but never forwards arbitrary response text", async () => {
    vi.spyOn(apiClient, "GET")
      .mockResolvedValueOnce({
        error: {
          code: "feature-stale-snapshot",
          message: "Private backend detail",
        },
        response: new Response(null, { status: 409 }),
      } as never)
      .mockResolvedValueOnce({
        error: {
          code: "private backend detail",
          message: "Another private detail",
        },
        response: new Response(null, { status: 503 }),
      } as never)
      .mockResolvedValueOnce({
        error: { code: "session-required" },
        response: new Response(null, { status: 401 }),
      } as never)
    const signal = new AbortController().signal
    await expect(api.get(featureId, signal)).rejects.toMatchObject({
      code: "feature-stale-snapshot",
      status: 409,
      message: "The request could not be completed",
    })
    await expect(api.get(featureId, signal)).rejects.toMatchObject({
      code: "unavailable",
      status: 503,
    })
    await expect(api.get(featureId, signal)).rejects.toBeInstanceOf(ApiFailure)
  })

  it("rejects incomplete approval authority and executable draft URLs", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockResolvedValueOnce(
      ok({
        ...feature,
        scope: { id: revisionId, digest: "digest", brief: "Product brief" },
      })
    )
    get.mockResolvedValueOnce(
      ok({
        ...feature,
        delivery: {
          requestId: operationId,
          snapshotId: revisionId,
          status: "published",
          draftUrl: "javascript:alert(1)",
        },
      })
    )
    const signal = new AbortController().signal
    await expect(api.get(featureId, signal)).rejects.toThrow()
    await expect(api.get(featureId, signal)).rejects.toThrow()
  })

  it("retains failed assignment evidence and the immutable diff identity", async () => {
    const attempt: api.FeatureAttempt = {
      id: operationId,
      role: "implementation",
      title: "Implement the view",
      assignedModel: "Terra",
      observedModel: null,
      status: "failed",
      activity: "Assigned model is unavailable",
      checkpoint: "abc123",
      canResume: false,
    }
    const diff: api.FeatureDiff = {
      baseOid: "base",
      headOid: "head",
      patchDigest: "digest",
      patch: "+ change",
      changedPaths: ["src/view.tsx"],
    }
    vi.spyOn(apiClient, "GET")
      .mockResolvedValueOnce(ok({ ...feature, attempts: [attempt] }))
      .mockResolvedValueOnce(ok(diff))
    const signal = new AbortController().signal
    expect((await api.get(featureId, signal)).attempts).toEqual([attempt])
    expect(await api.diff(featureId, signal)).toEqual(diff)
  })
})
