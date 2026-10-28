import { afterEach, expect, it, vi } from "vitest"

import { signInReason } from "../../src/features/auth/sign-in-view"

afterEach(() => vi.unstubAllGlobals())

it("rejects malformed active-session payloads and sends logout CSRF without browser storage", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(
      Response.json({
        user: { role: "admin" },
        csrfToken: "",
        expiresAt: "later",
      })
    )
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
  vi.stubGlobal("fetch", fetch)
  vi.resetModules()
  const { readSession, revokeSession } =
    await import("../../src/lib/auth/session-api")
  await expect(readSession(new AbortController().signal)).rejects.toThrow()
  await revokeSession("synthetic-csrf")
  const request = fetch.mock.calls[1]?.[0] as Request
  expect(request.url).toBe("http://localhost:3001/api/auth/logout")
  expect(request.headers.get("X-CSRF-Token")).toBe("synthetic-csrf")
  expect(request.credentials).toBe("include")
  expect(request.cache).toBe("no-store")
})

it("accepts only fixed sign-in outcome names instead of rendering arbitrary URL content", () => {
  expect(signInReason("pending")).toBe("pending")
  expect(signInReason("<script>")).toBeUndefined()
  expect(signInReason("toString")).toBeUndefined()
})
