import { afterEach, expect, it, vi } from "vitest"
import {
  claudeConnectionProblem,
  claudeSchema,
} from "../../src/features/agent-connections/claude-api"

afterEach(() => vi.unstubAllGlobals())

it("explains an unsupported or unavailable personal connection", () => {
  expect(
    claudeConnectionProblem({
      provider: "claude",
      status: "error",
      errorCode: "unsupported_method",
    })
  ).toContain("personal Claude account")
  expect(
    claudeConnectionProblem({ provider: "claude", status: "unavailable" })
  ).toContain("could not be checked")
  expect(
    claudeConnectionProblem({ provider: "claude", status: "disconnected" })
  ).toBeNull()
})

it("accepts only consistent Claude account projections", () => {
  expect(
    claudeSchema.safeParse({
      provider: "claude",
      status: "disconnected",
      account: null,
    }).success
  ).toBe(true)
  expect(
    claudeSchema.safeParse({ provider: "claude", status: "connected" }).success
  ).toBe(false)
  expect(
    claudeSchema.safeParse({
      provider: "codex",
      status: "connected",
      account: { plan: "plus" },
    }).success
  ).toBe(false)
  expect(
    claudeSchema.safeParse({
      provider: "claude",
      status: "connected",
      account: { plan: "x".repeat(65) },
    }).success
  ).toBe(false)
  expect(
    claudeSchema.safeParse({
      provider: "claude",
      status: "disconnected",
      challenge: { userCode: "secret" },
    }).success
  ).toBe(false)
})

it("accepts only Claude's own browser-code authorization challenge", () => {
  const verificationUrl =
    "https://claude.com/cai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&response_type=code&code_challenge_method=S256&state=fixture&code_challenge=fixture"
  const challenge = {
    operationId: "6a579df6-0551-41a3-9875-5db5aa7c1234",
    verificationUrl,
    userCode: null,
    expiresAt: "2099-01-01T00:00:00Z",
  }
  expect(
    claudeSchema.safeParse({
      provider: "claude",
      status: "connecting",
      challenge,
    }).success
  ).toBe(true)
  expect(
    claudeSchema.safeParse({
      provider: "claude",
      status: "connecting",
      challenge: {
        ...challenge,
        verificationUrl: `${verificationUrl}&redirect_uri=https://evil.test`,
      },
    }).success
  ).toBe(false)
  expect(
    claudeSchema.safeParse({ provider: "claude", status: "connecting" }).success
  ).toBe(false)
})

it("disconnects with the session CSRF token and no request body", async () => {
  const fetch = vi.fn().mockResolvedValue(
    Response.json({
      provider: "claude",
      status: "disconnected",
      account: null,
      challenge: null,
    })
  )
  vi.stubGlobal("fetch", fetch)
  vi.resetModules()
  const { logoutClaude } =
    await import("../../src/features/agent-connections/claude-api")
  await logoutClaude("synthetic-csrf")
  const request = fetch.mock.calls[0]![0] as Request
  expect(new URL(request.url).pathname).toBe(
    "/api/agent-connections/claude/logout"
  )
  expect(request.method).toBe("POST")
  expect(request.credentials).toBe("include")
  expect(request.cache).toBe("no-store")
  expect(request.headers.get("X-CSRF-Token")).toBe("synthetic-csrf")
  expect(request.body).toBeNull()
})
