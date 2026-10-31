import { describe, expect, it, vi } from "vitest"
import {
  latestReader,
  loadWorkspaceResources,
  mergeOlderConversations,
  runOutcome,
} from "./state"
import type { History } from "./api"

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}
describe("conversation snapshots", () => {
  it("ignores an older active refresh arriving after completion", async () => {
    const first = deferred<string>(),
      second = deferred<string>()
    const reader = latestReader(
      vi
        .fn()
        .mockReturnValueOnce(first.promise)
        .mockReturnValueOnce(second.promise)
    )
    const signal = new AbortController().signal
    const old = reader.read("conversation", signal),
      recent = reader.read("conversation", signal)
    second.resolve("completed")
    expect(await recent).toBe("completed")
    first.resolve("active")
    expect(await old).toBeUndefined()
  })
  it("invalidates results across conversation switches", async () => {
    const pending = deferred<string>(),
      reader = latestReader(() => pending.promise)
    const result = reader.read("first", new AbortController().signal)
    reader.invalidate()
    pending.resolve("old")
    expect(await result).toBeUndefined()
  })
  it("keeps conversations accessible when model discovery fails", async () => {
    const model = deferred<string[]>(),
      list = deferred<string[]>()
    const onList = vi.fn(),
      onModels = vi.fn(),
      onModelsError = vi.fn()
    const result = loadWorkspaceResources({
      signal: new AbortController().signal,
      list: () => list.promise,
      models: () => model.promise,
      onList,
      onModels,
      onListError: vi.fn(),
      onModelsError,
    })
    list.resolve(["chat"])
    await Promise.resolve()
    await Promise.resolve()
    expect(onList).toHaveBeenCalledWith(["chat"])
    model.reject(new Error("offline"))
    await result
    expect(onModels).not.toHaveBeenCalled()
    expect(onModelsError).toHaveBeenCalledOnce()
  })
  it("allows model discovery to be retried without losing the list", async () => {
    const loaded = vi.fn(),
      onError = vi.fn()
    const list = vi.fn().mockResolvedValue(["chat"])
    const models = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary"))
      .mockResolvedValueOnce(["model"])
    const options = {
      signal: new AbortController().signal,
      list,
      models,
      onList: vi.fn(),
      onModels: loaded,
      onListError: vi.fn(),
      onModelsError: onError,
    }
    await loadWorkspaceResources(options)
    expect(onError).toHaveBeenCalledOnce()
    await loadWorkspaceResources(options)
    expect(loaded).toHaveBeenCalledWith(["model"])
  })
  it.each([
    ["completed", "Codex finished."],
    ["failed", "Codex could not complete"],
    ["interrupted", "The run was interrupted"],
    ["uncertain", "The run may still be active"],
  ])("surfaces %s after reload", (status, message) => {
    const history = { runs: [{ status }] } as History
    expect(runOutcome(history, true)).toContain(message)
  })
})

describe("question receipts", () => {
  it("does not offer a new submission after a write without confirmation", async () => {
    const { questionReceipt } = await import("./state")
    expect(questionReceipt("pending", true)).toMatchObject({
      disabled: true,
      message: expect.stringContaining("Waiting"),
    })
    expect(questionReceipt("claimed", false)).toMatchObject({
      disabled: true,
      message: expect.stringContaining("Waiting"),
    })
    expect(questionReceipt("uncertain", false)).toMatchObject({
      disabled: true,
      message: expect.stringContaining("could not be confirmed"),
    })
    expect(questionReceipt("pending", false).disabled).toBe(false)
  })
})

describe("conversation pagination", () => {
  it("appends older pages without duplicating an already selected chat", () => {
    const item = (id: string) => ({ id, createdAt: "2026-01-01T00:00:00Z" })
    expect(
      mergeOlderConversations(
        [item("new"), item("selected")],
        [item("selected"), item("old")]
      )
    ).toEqual([item("new"), item("selected"), item("old")])
  })
})
