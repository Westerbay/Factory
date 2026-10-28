import { afterEach, expect, it, vi } from "vitest"
import type { components } from "../../src/lib/api/schema"

const user: components["schemas"]["AdminUser"] = {
  id: "d3d81ae5-5ee6-4a75-bb54-126bfe991643",
  displayName: "Synthetic Member",
  handle: "member",
  provider: "github",
  role: "member",
  status: "pending",
  version: 4,
}
afterEach(() => vi.unstubAllGlobals())

async function apiWith(response: Response) {
  const fetch = vi.fn().mockResolvedValue(response)
  vi.stubGlobal("fetch", fetch)
  vi.resetModules()
  return { ...(await import("../../src/features/admin-users/api")), fetch }
}

it("uses bounded status/cursor listing and rejects an unexpectedly oversized response", async () => {
  const api = await apiWith(
    Response.json({ items: Array.from({ length: 21 }, () => user) })
  )
  await expect(
    api.listUsers("pending", user.id, new AbortController().signal)
  ).rejects.toThrow()
  const request = api.fetch.mock.calls[0]?.[0] as Request
  const url = new URL(request.url)
  expect(url.searchParams.get("limit")).toBe("20")
  expect(url.searchParams.get("status")).toBe("pending")
  expect(url.searchParams.get("cursor")).toBe(user.id)
})

it("sends the current row version and session CSRF without client-controlled role/actor fields", async () => {
  const api = await apiWith(
    Response.json({ ...user, status: "active", version: 5 })
  )
  await api.changeAccess(user, "active", "synthetic-csrf")
  const request = api.fetch.mock.calls[0]?.[0] as Request
  expect(request.method).toBe("PATCH")
  expect(request.headers.get("X-CSRF-Token")).toBe("synthetic-csrf")
  expect(request.headers.get("Content-Type")).toContain("application/json")
  expect(request.credentials).toBe("include")
  expect(await request.json()).toEqual({ status: "active", expectedVersion: 4 })
})

it("preserves a version conflict for the UI to refresh instead of retrying the write", async () => {
  const api = await apiWith(
    Response.json(
      { code: "conflict", message: "Synthetic conflict", requestId: user.id },
      { status: 409 }
    )
  )
  await expect(
    api.changeAccess(user, "suspended", "synthetic-csrf")
  ).rejects.toMatchObject({ status: 409 })
  expect(api.fetch).toHaveBeenCalledOnce()
})

it("drops invalid filter/cursor URL input and keeps valid navigation state", async () => {
  const api = await apiWith(Response.json({ items: [] }))
  expect(api.parseUsersSearch({ status: "all", cursor: "../../" })).toEqual({
    status: undefined,
    cursor: undefined,
  })
  expect(api.parseUsersSearch({ status: "active", cursor: user.id })).toEqual({
    status: "active",
    cursor: user.id,
  })
})
