import { afterEach, describe, expect, it, vi } from "vitest"

import { readLiveness } from "../src/lib/api/client"
import { parseApiOrigin } from "../src/lib/api/config"

afterEach(() => vi.unstubAllGlobals())

describe("API origin", () => {
  it("accepts HTTPS and local HTTP but rejects insecure, credentialed and non-origin URLs", () => {
    expect(parseApiOrigin("/")).toBe("/")
    expect(parseApiOrigin()).toBe("http://localhost:3001")
    expect(parseApiOrigin("http://127.0.0.1:3001/")).toBe(
      "http://127.0.0.1:3001"
    )
    expect(parseApiOrigin("https://api.example.com")).toBe(
      "https://api.example.com"
    )
    for (const value of [
      "",
      "http://example.com",
      "https://name:secret@api.example.com",
      "https://api.example.com/path",
      "http://localhost.example.com",
      "http://name:secret@localhost:3001",
      "http://localhost:3001/api",
      "http://localhost:3001/?token=a",
      "http://localhost:3001/#hash",
    ]) {
      expect(() => parseApiOrigin(value)).toThrow(
        "must be /, an HTTPS origin, or a loopback HTTP origin"
      )
    }
  })
})

describe("health request", () => {
  it("requests the configured API with credentials, no cache and abort support", async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ status: "ok" }))
    vi.stubGlobal("fetch", fetch)
    await expect(readLiveness(new AbortController().signal)).resolves.toEqual({
      status: "ok",
    })
    expect(fetch).toHaveBeenCalledWith(
      "http://localhost:3001/health/live",
      expect.objectContaining({
        credentials: "include",
        cache: "no-store",
        redirect: "error",
        signal: expect.any(AbortSignal),
      })
    )
  })

  it("never reports an HTTP failure or an unexpected payload as a reachable Factory API", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ status: "ok" }, { status: 503 }))
      .mockResolvedValueOnce(Response.json({ status: "other" }))
    vi.stubGlobal("fetch", fetch)
    await expect(readLiveness(new AbortController().signal)).rejects.toThrow(
      "unavailable"
    )
    await expect(readLiveness(new AbortController().signal)).rejects.toThrow()
  })
})
