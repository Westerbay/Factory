import { afterEach, describe, expect, it, vi } from "vitest"

import { createAttachmentTray } from "../../src/features/conversations/attachment-tray"
import {
  maxAttachmentBytes,
  upload as uploadAttachment,
  type Attachment,
} from "../../src/features/conversations/attachments"

afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
})

const conversation = "9bf3cbec-9818-44c2-bd7a-a9c722b910e6"
let counter = 0
function staged(name: string): Attachment {
  counter++
  return {
    id: `00000000-0000-4000-8000-${String(counter).padStart(12, "0")}`,
    conversationId: conversation,
    source: "user",
    state: "staged",
    name,
    mediaType: "text/plain",
    kind: "text",
    size: 1,
    createdAt: "2026-01-01T00:00:00Z",
  }
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
function setup(
  upload?: (file: File, signal: AbortSignal) => Promise<Attachment>
) {
  const remove = vi.fn(async () => {})
  const conversationFor = vi.fn(async () => conversation)
  const tray = createAttachmentTray({
    conversation: conversationFor,
    upload: (_c, file, signal) =>
      upload ? upload(file, signal) : Promise.resolve(staged(file.name)),
    remove,
  })
  return { tray, remove, conversationFor }
}
const file = (name: string, size = 1) => {
  const value = new File(["x"], name)
  Object.defineProperty(value, "size", { value: size })
  return value
}

describe("attachment tray", () => {
  it("uploads files and exposes ready ids in order", async () => {
    const { tray, conversationFor } = setup()
    tray.add([file("a.txt"), file("b.txt")])
    expect(tray.uploading()).toBe(true)
    await settle()
    expect(tray.uploading()).toBe(false)
    expect(tray.getSnapshot().items.map((item) => item.status)).toEqual([
      "ready",
      "ready",
    ])
    expect(tray.readyIds()).toHaveLength(2)
    expect(conversationFor).toHaveBeenCalledTimes(2)
  })

  it("queues ten files within the server's four-transfer allowance", async () => {
    let active = 0
    let peak = 0
    const complete: (() => void)[] = []
    const { tray } = setup(async (value) => {
      active++
      peak = Math.max(peak, active)
      try {
        if (active > 4) throw new Error("server upload allowance exceeded")
        return await new Promise<Attachment>((resolve) => {
          complete.push(() => resolve(staged(value.name)))
        })
      } finally {
        active--
      }
    })
    tray.add(Array.from({ length: 10 }, (_, i) => file(`f${i}`)))
    for (let batch = 0; batch < 10; batch++) {
      await settle()
      const current = complete.splice(0).reverse()
      if (current.length === 0) break
      for (const finish of current) finish()
    }
    await settle()
    expect(peak).toBeLessThanOrEqual(4)
    expect(tray.readyIds()).toHaveLength(10)
    expect(
      tray.getSnapshot().items.map((item) => item.attachment?.name)
    ).toEqual(Array.from({ length: 10 }, (_, i) => `f${i}`))
  })

  it("recovers when an aborted upload holds its server permit during cleanup", async () => {
    vi.useFakeTimers()
    let activeOnServer = 0
    let busyResponses = 0
    const finish = new Map<string, () => void>()
    vi.spyOn(globalThis, "fetch").mockImplementation(async (_url, init) => {
      const value = init!.body as File
      const signal = init!.signal!
      if (activeOnServer === 4) {
        busyResponses++
        return new Response(null, { status: 503 })
      }
      activeOnServer++
      return new Promise<Response>((resolve, reject) => {
        function abort() {
          finish.delete(value.name)
          // Server cleanup outlives the browser's immediate fetch rejection.
          setTimeout(() => {
            activeOnServer--
          }, 1000)
          reject(signal.reason)
        }
        signal.addEventListener("abort", abort, { once: true })
        finish.set(value.name, () => {
          signal.removeEventListener("abort", abort)
          finish.delete(value.name)
          activeOnServer--
          resolve(Response.json(staged(value.name), { status: 201 }))
        })
      })
    })
    const tray = createAttachmentTray({
      conversation: async () => conversation,
      upload: (id, value, signal) =>
        uploadAttachment(id, value, "csrf", signal),
      remove: vi.fn(async () => {}),
    })
    tray.add(Array.from({ length: 6 }, (_, i) => file(`f${i}`)))
    await vi.advanceTimersByTimeAsync(0)
    await tray.remove(tray.getSnapshot().items[0]!.key)
    await vi.advanceTimersByTimeAsync(0)
    expect(busyResponses).toBeGreaterThan(0)
    await vi.advanceTimersByTimeAsync(2000)
    for (let wave = 0; wave < 3; wave++) {
      for (const complete of [...finish.values()]) complete()
      await vi.advanceTimersByTimeAsync(0)
    }
    expect(tray.getSnapshot().items.map((item) => item.status)).toEqual(
      Array(5).fill("ready")
    )
    expect(tray.readyIds()).toHaveLength(5)
    expect(activeOnServer).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it("does not leave a retry timer if removal precedes the busy response", async () => {
    vi.useFakeTimers()
    const tray: ReturnType<typeof createAttachmentTray> = createAttachmentTray({
      conversation: async () => conversation,
      upload: async () => {
        await tray.remove(tray.getSnapshot().items[0]!.key)
        throw Object.assign(new Error("busy"), { status: 503 })
      },
      remove: vi.fn(async () => {}),
    })
    tray.add([file("removed")])
    await vi.advanceTimersByTimeAsync(0)
    expect(tray.getSnapshot().items).toEqual([])
    expect(vi.getTimerCount()).toBe(0)
  })

  it("stops retrying persistent unavailability and exposes the failure", async () => {
    vi.useFakeTimers()
    const send = vi.fn(async () => {
      throw Object.assign(new Error("busy"), { status: 503 })
    })
    const { tray } = setup(send)
    tray.add([file("busy")])
    await vi.runAllTimersAsync()
    expect(send).toHaveBeenCalledTimes(9)
    expect(tray.getSnapshot().items[0]!.status).toBe("failed")
    expect(vi.getTimerCount()).toBe(0)
  })

  it.each([401, 403, 409, 413, 429, 500])(
    "does not retry an upload rejected with %i",
    async (status) => {
      vi.useFakeTimers()
      const send = vi.fn(async () => {
        throw Object.assign(new Error("rejected"), { status })
      })
      const { tray } = setup(send)
      tray.add([file("rejected")])
      await vi.runAllTimersAsync()
      expect(send).toHaveBeenCalledTimes(1)
      expect(tray.getSnapshot().items[0]!.status).toBe("failed")
      expect(vi.getTimerCount()).toBe(0)
    }
  )

  it("does not replay a request with an unknown network outcome", async () => {
    vi.useFakeTimers()
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockRejectedValue(new TypeError("network failure"))
    const tray = createAttachmentTray({
      conversation: async () => conversation,
      upload: (id, value, signal) =>
        uploadAttachment(id, value, "csrf", signal),
      remove: vi.fn(async () => {}),
    })
    tray.add([file("unknown")])
    await vi.runAllTimersAsync()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(tray.getSnapshot().items[0]!.status).toBe("failed")
  })

  it("clears an upload waiting to retry and leaves the next draft usable", async () => {
    vi.useFakeTimers()
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(null, { status: 503 }))
    const tray = createAttachmentTray({
      conversation: async () => conversation,
      upload: (id, value, signal) =>
        uploadAttachment(id, value, "csrf", signal),
      remove: vi.fn(async () => {}),
    })
    tray.add([file("cancelled")])
    await vi.advanceTimersByTimeAsync(0)
    tray.clear()
    await vi.runAllTimersAsync()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(tray.getSnapshot().items).toEqual([])
    fetchMock.mockResolvedValueOnce(
      Response.json(staged("next"), { status: 201 })
    )
    tray.add([file("next")])
    await vi.advanceTimersByTimeAsync(0)
    expect(tray.readyIds()).toHaveLength(1)
    expect(vi.getTimerCount()).toBe(0)
  })

  it("never uploads queued files removed or cleared before their turn", async () => {
    const started: string[] = []
    const complete: (() => void)[] = []
    const { tray, remove } = setup((value) => {
      started.push(value.name)
      return new Promise<Attachment>((resolve) => {
        complete.push(() => resolve(staged(value.name)))
      })
    })
    tray.add(Array.from({ length: 6 }, (_, i) => file(`f${i}`)))
    await settle()
    await tray.remove(tray.getSnapshot().items[4]!.key)
    tray.clear()
    for (const finish of complete.splice(0)) finish()
    await settle()
    expect(started).toEqual(["f0", "f1", "f2", "f3"])
    expect(remove).toHaveBeenCalledTimes(4)
    expect(tray.getSnapshot().items).toEqual([])
    tray.add([file("next")])
    await settle()
    expect(started.at(-1)).toBe("next")
    for (const finish of complete.splice(0)) finish()
    await settle()
    expect(tray.readyIds()).toHaveLength(1)
  })

  it("does not start a removed upload after conversation creation settles", async () => {
    let resolve!: (value: string) => void
    const upload = vi.fn(async () => staged("removed"))
    const tray = createAttachmentTray({
      conversation: () => new Promise<string>((done) => (resolve = done)),
      upload,
      remove: vi.fn(async () => {}),
    })
    tray.add([file("removed")])
    await tray.remove(tray.getSnapshot().items[0]!.key)
    resolve(conversation)
    await settle()
    expect(upload).not.toHaveBeenCalled()
  })

  it("rejects oversized files and files beyond ten", async () => {
    const { tray } = setup()
    tray.add([file("big.bin", maxAttachmentBytes + 1)])
    expect(tray.getSnapshot().rejections).toEqual([
      { name: "big.bin", reason: "too-large" },
    ])
    tray.add([file("ok.bin", maxAttachmentBytes)])
    tray.add(Array.from({ length: 10 }, (_, i) => file(`f${i}`)))
    const snapshot = tray.getSnapshot()
    expect(snapshot.items).toHaveLength(10)
    expect(snapshot.rejections).toEqual([{ name: "f9", reason: "too-many" }])
    await settle()
  })

  it("marks failures and lets them be retried by removal and re-adding", async () => {
    const { tray } = setup(() => Promise.reject(new Error("boom")))
    tray.add([file("a.txt")])
    await settle()
    expect(tray.getSnapshot().items[0]!.status).toBe("failed")
    expect(tray.readyIds()).toEqual([])
    await tray.remove(tray.getSnapshot().items[0]!.key)
    expect(tray.getSnapshot().items).toEqual([])
  })

  it("deletes a staged upload on removal and aborts one in flight", async () => {
    const { tray, remove } = setup()
    tray.add([file("a.txt")])
    await settle()
    await tray.remove(tray.getSnapshot().items[0]!.key)
    expect(remove).toHaveBeenCalledTimes(1)

    let resolve!: (value: Attachment) => void
    const pending = setup(
      () => new Promise<Attachment>((res) => (resolve = res))
    )
    pending.tray.add([file("slow.txt")])
    await settle()
    await pending.tray.remove(pending.tray.getSnapshot().items[0]!.key)
    resolve(staged("slow.txt"))
    await settle()
    expect(pending.remove).toHaveBeenCalledTimes(1)
    expect(pending.tray.getSnapshot().items).toEqual([])
  })

  it("retries a busy 503 upload instead of failing the file", async () => {
    vi.useFakeTimers()
    try {
      let calls = 0
      const { tray } = setup(async (f) => {
        if (++calls < 3) throw Object.assign(new Error("busy"), { status: 503 })
        return staged(f.name)
      })
      tray.add([file("a.txt")])
      await vi.advanceTimersByTimeAsync(5000)
      expect(calls).toBe(3)
      expect(tray.getSnapshot().items[0]!.status).toBe("ready")
    } finally {
      vi.useRealTimers()
    }
  })

  it("fails on non-busy errors without retrying", async () => {
    let calls = 0
    const { tray } = setup(async () => {
      calls++
      throw Object.assign(new Error("bad"), { status: 400 })
    })
    tray.add([file("a.txt")])
    await settle()
    expect(calls).toBe(1)
    expect(tray.getSnapshot().items[0]!.status).toBe("failed")
  })

  it("clears sent items without deleting them and notifies subscribers", async () => {
    const { tray, remove } = setup()
    const listener = vi.fn()
    tray.subscribe(listener)
    tray.add([file("a.txt")])
    await settle()
    tray.clear()
    expect(tray.getSnapshot().items).toEqual([])
    expect(remove).not.toHaveBeenCalled()
    expect(listener).toHaveBeenCalled()
  })
})
