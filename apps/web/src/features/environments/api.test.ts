import { afterEach, expect, it, vi } from "vitest"

afterEach(() => vi.unstubAllGlobals())

it("shows known storage failures and ignores unknown error codes", async () => {
  const { environmentProblem } = await import("./api")
  expect(environmentProblem("volume_missing")).toContain(
    "will not be replaced automatically"
  )
  expect(environmentProblem("toString")).toBeUndefined()
  expect(environmentProblem("unknown_code")).toBeUndefined()
})

async function apiWith(response: Response) {
  const fetch = vi.fn().mockResolvedValue(response)
  vi.stubGlobal("fetch", fetch)
  vi.resetModules()
  return { ...(await import("./api")), fetch }
}

it.each(["start", "stop", "heartbeat"] as const)(
  "sends %s with credentials, CSRF and no body",
  async (action) => {
    const api = await apiWith(
      Response.json(
        { status: "starting" },
        { status: action === "heartbeat" ? 200 : 202 }
      )
    )
    await api.changeEnvironment(
      action,
      "synthetic-csrf",
      new AbortController().signal
    )
    const request = api.fetch.mock.calls[0]![0] as Request
    expect(new URL(request.url).pathname).toBe(`/api/environment/${action}`)
    expect(request.method).toBe("POST")
    expect(request.credentials).toBe("include")
    expect(request.cache).toBe("no-store")
    expect(request.headers.get("X-CSRF-Token")).toBe("synthetic-csrf")
    expect(request.body).toBeNull()
    expect(await request.text()).toBe("")
  }
)

it.each([
  { status: "unknown" },
  { status: "running", observedAt: "yesterday" },
])("rejects untrusted environment response %j", async (body) => {
  const api = await apiWith(Response.json(body))
  await expect(
    api.readEnvironment(new AbortController().signal)
  ).rejects.toThrow()
})

it("accepts unavailable and nullable dates without sending a mutation", async () => {
  const body = {
    status: "unavailable",
    observedAt: null,
    idleExpiresAt: null,
    errorCode: "unavailable",
  }
  const api = await apiWith(Response.json(body))
  await expect(
    api.readEnvironment(new AbortController().signal)
  ).resolves.toEqual(body)
  const request = api.fetch.mock.calls[0]![0] as Request
  expect(request.method).toBe("GET")
  expect(new URL(request.url).pathname).toBe("/api/environment")
})
