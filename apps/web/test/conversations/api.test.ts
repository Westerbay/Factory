import { afterEach, describe, expect, it, vi } from "vitest"

import { apiClient, ApiFailure } from "../../src/lib/api/client"
import {
  cancel,
  ContextTransferFailure,
  create,
  list,
  listRecoverable,
  recoverModelProbe,
  start,
  stream,
} from "../../src/features/conversations/api"

const userRun = "f7eae12b-e8f7-4ee3-a27c-a9c84a12962b"
const conversation = "9bf3cbec-9818-44c2-bd7a-a9c722b910e6"
afterEach(() => vi.restoreAllMocks())

it("distinguishes a transfer limit from ordinary conversation conflicts", async () => {
  const post = vi.spyOn(apiClient, "POST")
  post.mockResolvedValueOnce({
    error: { code: "ContextLimit" },
    response: new Response(null, { status: 409 }),
  } as never)
  post.mockResolvedValueOnce({
    error: { code: "Conflict" },
    response: new Response(null, { status: 409 }),
  } as never)
  const body = {
    operationId: userRun,
    model: "factory:claude:sonnet",
    input: "hello",
  }
  const send = () =>
    start(conversation, body, "csrf", new AbortController().signal)
  await expect(send()).rejects.toBeInstanceOf(ContextTransferFailure)
  await expect(send()).rejects.toBeInstanceOf(ApiFailure)
})

describe("conversation pages", () => {
  it("creates a project-scoped chat and preserves its association in the response", async () => {
    const projectId = "b8903ea3-3e05-4780-a2b6-d05b9227b078"
    const post = vi.spyOn(apiClient, "POST")
    post.mockResolvedValueOnce({
      data: { id: conversation, createdAt: "2026-01-01T00:00:00Z", projectId },
      response: new Response(null, { status: 200 }),
    } as never)
    const signal = new AbortController().signal
    expect((await create("csrf", signal, projectId)).projectId).toBe(projectId)
    expect(post).toHaveBeenCalledWith("/api/conversations", {
      params: { header: { "X-CSRF-Token": "csrf" } },
      body: { projectId },
      signal,
    })
  })

  it("requests an older page using the cursor and validates the response", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockResolvedValueOnce({
      data: {
        items: [{ id: conversation, createdAt: "2026-01-01T00:00:00Z" }],
        nextCursor: null,
      },
      response: new Response(null, { status: 200 }),
    } as never)
    const signal = new AbortController().signal
    expect(await list(signal, conversation)).toEqual({
      items: [{ id: conversation, createdAt: "2026-01-01T00:00:00Z" }],
      nextCursor: null,
    })
    expect(get).toHaveBeenCalledWith("/api/conversations", {
      params: { query: { before: conversation } },
      signal,
    })
  })
})

describe("conversation recovery", () => {
  it("accepts an empty recovery list and validated owned identifiers", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockResolvedValueOnce({
      data: [],
      response: new Response(null, { status: 200 }),
    } as never)
    get.mockResolvedValueOnce({
      data: [
        { conversationId: conversation, runId: userRun, launchSettled: true },
      ],
      response: new Response(null, { status: 200 }),
    } as never)
    expect(await listRecoverable(new AbortController().signal)).toEqual([])
    expect(await listRecoverable(new AbortController().signal)).toEqual([
      { conversationId: conversation, runId: userRun, launchSettled: true },
    ])
  })
  it("preserves an unconfirmed launch and rejects a missing launch state", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockResolvedValueOnce({
      data: [
        { conversationId: conversation, runId: userRun, launchSettled: false },
      ],
      response: new Response(null, { status: 200 }),
    } as never)
    get.mockResolvedValueOnce({
      data: [{ conversationId: conversation, runId: userRun }],
      response: new Response(null, { status: 200 }),
    } as never)
    expect(await listRecoverable(new AbortController().signal)).toEqual([
      { conversationId: conversation, runId: userRun, launchSettled: false },
    ])
    await expect(
      listRecoverable(new AbortController().signal)
    ).rejects.toThrow()
  })
  it("recovers model capacity explicitly with the session CSRF token", async () => {
    const post = vi.spyOn(apiClient, "POST")
    post.mockResolvedValueOnce({
      response: new Response(null, { status: 204 }),
    } as never)
    post.mockResolvedValueOnce({
      response: new Response(null, { status: 409 }),
    } as never)
    const signal = new AbortController().signal
    await recoverModelProbe("csrf", signal)
    expect(post).toHaveBeenCalledWith(
      "/api/conversations/model-probe/recover",
      {
        params: { header: { "X-CSRF-Token": "csrf" } },
        signal,
      }
    )
    await expect(recoverModelProbe("csrf", signal)).rejects.toBeInstanceOf(
      ApiFailure
    )
  })
  it("does not interpret an unavailable or malformed response as no work", async () => {
    const get = vi.spyOn(apiClient, "GET")
    get.mockResolvedValueOnce({
      response: new Response(null, { status: 503 }),
    } as never)
    get.mockResolvedValueOnce({
      data: [{ conversationId: "bad", runId: userRun }],
      response: new Response(null, { status: 200 }),
    } as never)
    await expect(
      listRecoverable(new AbortController().signal)
    ).rejects.toBeInstanceOf(ApiFailure)
    await expect(
      listRecoverable(new AbortController().signal)
    ).rejects.toThrow()
  })
})

describe("conversation mutation headers", () => {
  it("sends the CSRF token without setting the browser-controlled Origin", async () => {
    const requests: Request[] = []
    const capture = {
      onRequest({ request }: { request: Request }) {
        requests.push(request)
        return new Response(null, { status: 204 })
      },
    }
    apiClient.use(capture)
    try {
      await cancel(
        conversation,
        userRun,
        "fixture-csrf",
        new AbortController().signal
      )
      expect(requests).toHaveLength(1)
      expect(requests[0]?.headers.get("X-CSRF-Token")).toBe("fixture-csrf")
      expect(requests[0]?.headers.has("Origin")).toBe(false)
    } finally {
      apiClient.eject(capture)
    }
  })
})

describe("conversation stream", () => {
  it("parses fragmented CRLF events without replaying any input", async () => {
    const encode = new TextEncoder()
    const body = new ReadableStream({
      start(controller) {
        controller.enqueue(
          encode.encode(`data: {"kind":"delta","runId":"${userRun}",`)
        )
        controller.enqueue(encode.encode('"text":"hello"}\r\n\r\n'))
        controller.close()
      },
    })
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(body, { status: 200 }))
    const observed = vi.fn()
    await stream(conversation, userRun, new AbortController().signal, observed)
    expect(observed).toHaveBeenCalledExactlyOnceWith({
      kind: "delta",
      runId: userRun,
      text: "hello",
    })
    expect(fetchSpy.mock.calls[0]?.[1]).toMatchObject({
      credentials: "include",
      cache: "no-store",
    })
    expect(fetchSpy.mock.calls[0]?.[1]?.method).toBeUndefined()
  })
  it("rejects another run's events", async () => {
    const body = `data: {"kind":"refresh","runId":"${conversation}"}\n\n`
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(body, { status: 200 })
    )
    await expect(
      stream(conversation, userRun, new AbortController().signal, vi.fn())
    ).rejects.toThrow("Unexpected run event")
  })
})
