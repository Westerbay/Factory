import { afterEach, describe, expect, it, vi } from "vitest"

import { apiClient, ApiFailure } from "../../src/lib/api/client"
import {
  contentUrl,
  hideFactoryFiles,
  loadText,
  maxAttachmentBytes,
  remove,
  upload,
} from "../../src/features/conversations/attachments"

const conversation = "9bf3cbec-9818-44c2-bd7a-a9c722b910e6"
const attachment = "f7eae12b-e8f7-4ee3-a27c-a9c84a12962b"
const view = {
  id: attachment,
  conversationId: conversation,
  source: "user",
  state: "staged",
  name: "a b.png",
  mediaType: "image/png",
  kind: "image",
  size: 3,
  createdAt: "2026-01-01T00:00:00Z",
}
afterEach(() => vi.restoreAllMocks())

describe("attachment API", () => {
  it("uploads raw bytes with credentials and CSRF, and validates the response", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(Response.json(view, { status: 201 }))
    const file = new File(["abc"], "a b.png")
    const result = await upload(
      conversation,
      file,
      "csrf",
      new AbortController().signal
    )
    expect(result.id).toBe(attachment)
    const [url, init] = fetchMock.mock.calls[0]!
    expect(String(url)).toContain(
      `/api/conversations/${conversation}/attachments?name=a%20b.png`
    )
    expect(init).toMatchObject({
      method: "POST",
      credentials: "include",
      body: file,
      headers: { "X-CSRF-Token": "csrf" },
    })
  })

  it("rejects failures and malformed responses", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
    const signal = new AbortController().signal
    const file = new File(["abc"], "a.txt")
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 413 }))
    await expect(upload(conversation, file, "c", signal)).rejects.toMatchObject(
      { status: 413 }
    )
    fetchMock.mockResolvedValueOnce(
      Response.json({ ...view, kind: "executable" }, { status: 201 })
    )
    await expect(upload(conversation, file, "c", signal)).rejects.toThrow()
  })

  it("removes a staged attachment with the CSRF token", async () => {
    const del = vi.spyOn(apiClient, "DELETE").mockResolvedValue({
      response: new Response(null, { status: 204 }),
    } as never)
    await remove(conversation, attachment, "csrf", new AbortController().signal)
    expect(del).toHaveBeenCalledWith(
      "/api/conversations/{id}/attachments/{attachment}",
      expect.objectContaining({
        params: {
          path: { id: conversation, attachment },
          header: { "X-CSRF-Token": "csrf" },
        },
      })
    )
    del.mockResolvedValue({
      response: new Response(null, { status: 409 }),
    } as never)
    await expect(
      remove(conversation, attachment, "csrf", new AbortController().signal)
    ).rejects.toBeInstanceOf(ApiFailure)
  })

  it("builds read and download URLs", () => {
    expect(contentUrl(conversation, attachment)).toMatch(
      new RegExp(`/attachments/${attachment}/content$`)
    )
    expect(contentUrl(conversation, attachment, true)).toMatch(
      /content\?download=1$/
    )
  })

  it("loads bounded text with a Range request", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("hello world", { status: 206 }))
    expect(
      await loadText(conversation, attachment, 5, new AbortController().signal)
    ).toBe("hello")
    expect(fetchMock.mock.calls[0]![1]).toMatchObject({
      headers: { Range: "bytes=0-4" },
    })
  })

  it("keeps the documented limit", () => {
    expect(maxAttachmentBytes).toBe(50 * 1024 * 1024)
  })
})

describe("factory-files blocks", () => {
  it("hides complete blocks and keeps the surrounding text", () => {
    expect(
      hideFactoryFiles(
        "Here.\n\n```factory-files\n/a.png\n/b.pdf\n```\n\nDone."
      )
    ).toBe("Here.\n\nDone.")
  })
  it("hides a block still streaming", () => {
    expect(hideFactoryFiles("Here.\n\n```factory-files\n/a.p")).toBe(
      "Here.\n\n"
    )
    expect(hideFactoryFiles("Here.\n```factory-files")).toBe("Here.\n")
  })
  it("leaves lookalikes visible", () => {
    for (const text of [
      "```factory-files-extra\n/a\n```",
      "inline ```factory-files``` mention",
      "    ```factory-files\n/indented\n    ```",
    ])
      expect(hideFactoryFiles(text)).toBe(text)
  })
})
