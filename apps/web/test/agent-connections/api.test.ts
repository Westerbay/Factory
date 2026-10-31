import { afterEach, expect, it, vi } from "vitest"
import {
  connectionSchema,
  isOfficialVerificationUrl,
  connectionProblem,
} from "../../src/features/agent-connections/api"

afterEach(() => vi.unstubAllGlobals())
it.each([
  "https://auth.openai.com.evil.test/codex/device",
  "https://auth.openai.com@evil.test/codex/device",
  "https://user@auth.openai.com/codex/device",
  "http://auth.openai.com/codex/device",
  "https://auth.openai.com:444/codex/device",
  "https://auth.openai.com/codex/device?token=secret",
  "https://auth.openai.com/codex/device#secret",
  "https://auth.openai.com/other",
  "javascript:alert(1)",
])("rejects untrusted verification URL %s", (url) => {
  expect(isOfficialVerificationUrl(url)).toBe(false)
})
it("accepts only the official URL and rejects inconsistent or unbounded private data", () => {
  expect(
    isOfficialVerificationUrl("https://auth.openai.com/codex/device")
  ).toBe(true)
  expect(
    connectionSchema.safeParse({ provider: "codex", status: "connecting" })
      .success
  ).toBe(false)
  expect(
    connectionSchema.safeParse({
      provider: "codex",
      status: "connected",
      account: { email: null, plan: "x".repeat(65) },
    }).success
  ).toBe(false)
  expect(
    connectionSchema.safeParse({
      provider: "codex",
      status: "disconnected",
      account: { email: null, plan: "plus" },
    }).success
  ).toBe(false)
  expect(connectionProblem("toString")).toBeUndefined()
  expect(connectionProblem("unsupported_method")).toContain("API key")
})
async function apiWith(body: unknown, status = 200) {
  const fetch = vi.fn().mockResolvedValue(Response.json(body, { status }))
  vi.stubGlobal("fetch", fetch)
  vi.resetModules()
  return {
    ...(await import("../../src/features/agent-connections/api")),
    fetch,
  }
}
it.each(["login", "logout"] as const)(
  "sends %s with credentials and CSRF but no body",
  async (type) => {
    const api = await apiWith(
      { provider: "codex", status: "disconnected" },
      type === "login" ? 202 : 200
    )
    await api.changeConnection(
      { type },
      "synthetic-csrf",
      new AbortController().signal
    )
    const request = api.fetch.mock.calls[0]![0] as Request
    expect(new URL(request.url).pathname).toBe(
      `/api/agent-connections/codex/${type}`
    )
    expect(request.method).toBe("POST")
    expect(request.credentials).toBe("include")
    expect(request.cache).toBe("no-store")
    expect(request.redirect).toBe("error")
    expect(request.headers.get("X-CSRF-Token")).toBe("synthetic-csrf")
    expect(request.body).toBeNull()
  }
)
it("cancels using only the Factory operation ID", async () => {
  const api = await apiWith({
    provider: "codex",
    status: "error",
    errorCode: "cancelled",
  })
  const operationId = "1e1b1d41-4221-44b9-a8e1-d5eaf5210355"
  await api.changeConnection(
    { type: "cancel", operationId },
    "synthetic-csrf",
    new AbortController().signal
  )
  const request = api.fetch.mock.calls[0]![0] as Request
  expect(new URL(request.url).pathname).toBe(
    "/api/agent-connections/codex/login/cancel"
  )
  expect(await request.json()).toEqual({ operationId })
})
it("reads unavailable state without a mutation or private cache", async () => {
  const api = await apiWith({
    provider: "codex",
    status: "unavailable",
    account: null,
    challenge: null,
    observedAt: null,
  })
  expect((await api.readConnection(new AbortController().signal)).status).toBe(
    "unavailable"
  )
  const request = api.fetch.mock.calls[0]![0] as Request
  expect(request.method).toBe("GET")
  expect(request.cache).toBe("no-store")
})
