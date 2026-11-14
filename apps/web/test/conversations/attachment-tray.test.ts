import { describe, expect, it, vi } from "vitest"

import { createAttachmentTray } from "../../src/features/conversations/attachment-tray"
import {
  maxAttachmentBytes,
  type Attachment,
} from "../../src/features/conversations/attachments"

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
