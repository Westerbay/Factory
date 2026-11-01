import { afterEach, describe, expect, it, vi } from "vitest"

import { apiClient } from "../../src/lib/api/client"
import {
  connect,
  ensureConnection,
  hasRecoverableWork,
  importRepository,
  isDefiniteImportRejection,
  list,
  projectProblem,
  ProjectFailure,
  remainingAfterRecovery,
  remove,
  repositories,
  verifyAddress,
} from "../../src/features/projects/api"

const project = "9bf3cbec-9818-44c2-bd7a-a9c722b910e6"
afterEach(() => vi.restoreAllMocks())
const ok = (data: unknown) =>
  ({ data, response: new Response(null, { status: 200 }) }) as never

describe("project API", () => {
  it("automatically completes GitHub setup when the environment is ready", async () => {
    const get = vi
      .spyOn(apiClient, "GET")
      .mockResolvedValueOnce(ok({ status: "disconnected" }))
      .mockResolvedValueOnce(
        ok({
          status: "connected",
          login: "fixture",
          email: "fixture@example.test",
        })
      )
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce({
      response: new Response(null, { status: 204 }),
    } as never)
    const signal = new AbortController().signal
    expect((await ensureConnection("csrf", signal)).status).toBe("connected")
    expect(post).toHaveBeenCalledWith("/api/projects/setup", {
      params: { header: { "X-CSRF-Token": "csrf" } },
      signal,
    })
    expect(get).toHaveBeenCalledTimes(2)
  })
  it("keeps the reconnect option when an existing session has no repository token", async () => {
    vi.spyOn(apiClient, "GET").mockResolvedValueOnce(
      ok({ status: "disconnected" })
    )
    vi.spyOn(apiClient, "POST").mockResolvedValueOnce({
      error: { code: "provider_unauthorized" },
      response: new Response(null, { status: 409 }),
    } as never)
    expect(
      (await ensureConnection("csrf", new AbortController().signal)).status
    ).toBe("disconnected")
  })
  it("verifies the original address without rewriting the local remote", async () => {
    const get = vi
      .spyOn(apiClient, "GET")
      .mockResolvedValueOnce(ok({ status: "changed" }))
    const signal = new AbortController().signal
    expect(await verifyAddress(project, signal)).toBe("changed")
    expect(get).toHaveBeenCalledWith("/api/projects/{id}/verification", {
      params: { path: { id: project } },
      signal,
    })
    get.mockResolvedValueOnce({
      error: { code: "provider_forbidden" },
      response: new Response(null, { status: 409 }),
    } as never)
    await expect(verifyAddress(project, signal)).rejects.toBeInstanceOf(
      ProjectFailure
    )
  })
  it("posts a repository page with CSRF and validates importable results", async () => {
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce(
      ok({
        items: [{ id: 24, fullName: "owner/repo", private: true }],
        nextPage: 2,
      })
    )
    const signal = new AbortController().signal
    expect(await repositories(1, "csrf", signal)).toEqual({
      items: [{ id: 24, fullName: "owner/repo", private: true }],
      nextPage: 2,
    })
    expect(post).toHaveBeenCalledWith("/api/projects/repositories", {
      params: { header: { "X-CSRF-Token": "csrf" } },
      body: { page: 1 },
      signal,
    })
  })
  it("sends a CSRF-protected project deletion and preserves the in-use error", async () => {
    const del = vi.spyOn(apiClient, "DELETE").mockResolvedValueOnce({
      response: new Response(null, { status: 204 }),
    } as never)
    const signal = new AbortController().signal
    await remove(project, "csrf", signal)
    expect(del).toHaveBeenCalledWith("/api/projects/{id}", {
      params: { path: { id: project }, header: { "X-CSRF-Token": "csrf" } },
      signal,
    })
    del.mockResolvedValueOnce({
      error: { code: "project_in_use" },
      response: new Response(null, { status: 409 }),
    } as never)
    const error = await remove(project, "csrf", signal).catch(
      (value: unknown) => value
    )
    expect(projectProblem(error)).toContain("active or uncertain runs")
  })
  it("imports only the selected repository ID and page", async () => {
    const post = vi.spyOn(apiClient, "POST").mockResolvedValueOnce(
      ok({
        id: project,
        fullName: "owner/repo",
        state: "importing",
        renamedSinceClone: false,
      })
    )
    const signal = new AbortController().signal
    expect((await importRepository(24, 2, "csrf", signal)).state).toBe(
      "importing"
    )
    expect(post).toHaveBeenCalledWith("/api/projects", {
      params: { header: { "X-CSRF-Token": "csrf" } },
      body: { repositoryId: 24, page: 2 },
      signal,
    })
  })
  it("preserves uncertain state and surfaces provider denial distinctly", async () => {
    const get = vi.spyOn(apiClient, "GET").mockResolvedValueOnce(
      ok({
        items: [
          {
            id: project,
            fullName: "owner/repo",
            state: "uncertain",
            renamedSinceClone: false,
          },
        ],
        nextCursor: null,
      })
    )
    expect((await list(new AbortController().signal)).items[0]?.state).toBe(
      "uncertain"
    )
    get.mockResolvedValueOnce({
      error: { code: "provider_forbidden" },
      response: new Response(null, { status: 409 }),
    } as never)
    const error = await list(new AbortController().signal).catch(
      (value: unknown) => value
    )
    expect(error).toBeInstanceOf(ProjectFailure)
    expect(projectProblem(error)).toContain("organization")
  })
  it("keeps provider rejection distinct from an uncertain transport failure", () => {
    const rejected = new ProjectFailure(409, "provider_unauthorized")
    expect(isDefiniteImportRejection(rejected)).toBe(true)
    expect(projectProblem(rejected)).toContain("Reconnect GitHub")
    const collision = new ProjectFailure(409, "project_directory_exists")
    expect(isDefiniteImportRejection(collision)).toBe(true)
    expect(projectProblem(collision)).toContain("already exists in Documents")
    expect(isDefiniteImportRejection(new TypeError("network failed"))).toBe(
      false
    )
    expect(
      isDefiniteImportRejection(new ProjectFailure(503, "unavailable"))
    ).toBe(false)
  })
  it("preserves a rename warning returned after repository metadata sync", async () => {
    vi.spyOn(apiClient, "GET").mockResolvedValueOnce(
      ok({
        items: [
          {
            id: project,
            fullName: "new-owner/repo",
            state: "ready",
            renamedSinceClone: true,
          },
        ],
        nextCursor: null,
      })
    )
    expect((await list(new AbortController().signal)).items[0]).toMatchObject({
      fullName: "new-owner/repo",
      renamedSinceClone: true,
    })
  })
  it("reloads after the last recovered import and rejects an unknown next batch", async () => {
    const get = vi.spyOn(apiClient, "GET")
    const signal = new AbortController().signal
    const pending = [{ projectId: project }]
    expect(await remainingAfterRecovery(pending, signal)).toEqual(pending)
    expect(get).not.toHaveBeenCalled()
    get.mockResolvedValueOnce(ok([{ projectId: project }]))
    expect(await remainingAfterRecovery([], signal)).toEqual(pending)
    get.mockResolvedValueOnce(ok([]))
    expect(await remainingAfterRecovery([], signal)).toEqual([])
    get.mockResolvedValueOnce({
      response: new Response(null, { status: 503 }),
    } as never)
    await expect(remainingAfterRecovery([], signal)).rejects.toBeInstanceOf(
      ProjectFailure
    )
  })
  it("rejects an authorization URL outside GitHub", async () => {
    const post = vi.spyOn(apiClient, "POST")
    post.mockResolvedValueOnce(
      ok({ authorizationUrl: "https://example.invalid/login/oauth/authorize" })
    )
    post.mockResolvedValueOnce(
      ok({
        authorizationUrl:
          "https://github.com/login/oauth/authorize?client_id=fixture",
      })
    )
    const signal = new AbortController().signal
    await expect(connect("csrf", signal)).rejects.toThrow(
      "Invalid authorization URL"
    )
    await expect(connect("csrf", signal)).resolves.toContain(
      "github.com/login/oauth/authorize"
    )
  })
  it("blocks startup if either recovery list fails, and detects interrupted imports", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockImplementation((path) =>
      Promise.resolve(
        path === "/api/projects/recoverable"
          ? ok([{ projectId: project }])
          : ok([])
      )
    )
    expect(await hasRecoverableWork(new AbortController().signal)).toBe(true)
    get.mockImplementation((path) =>
      Promise.resolve(
        path === "/api/projects/recoverable"
          ? ({ response: new Response(null, { status: 503 }) } as never)
          : ok([])
      )
    )
    await expect(
      hasRecoverableWork(new AbortController().signal)
    ).rejects.toBeInstanceOf(ProjectFailure)
  })
})
