import { describe, expect, it, vi } from "vitest"
import {
  latestReader,
  mergeOlderConversations,
  runOutcome,
  selectedEffort,
} from "../../src/features/conversations/state"
import type { History } from "../../src/features/conversations/api"

describe("model effort selection", () => {
  const model = {
    id: "model",
    name: "Model",
    effortLevels: ["low", "medium", "high"],
    defaultEffort: "low",
  }
  it("keeps a supported choice and otherwise selects an advertised level", () => {
    expect(selectedEffort(model, "high")).toBe("high")
    expect(selectedEffort(model, "")).toBe("low")
    expect(selectedEffort({ ...model, defaultEffort: null }, "")).toBe("medium")
    expect(
      selectedEffort({ ...model, defaultEffort: "invalid" }, "removed")
    ).toBe("medium")
    expect(
      selectedEffort(
        { ...model, defaultEffort: null, effortLevels: ["high"] },
        ""
      )
    ).toBe("high")
    expect(selectedEffort({ ...model, effortLevels: [] }, "")).toBe("")
    expect(selectedEffort(undefined, "high")).toBe("")
  })
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
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
  it.each([
    ["completed", "Assistant finished."],
    ["failed", "Assistant could not complete"],
    ["interrupted", "The run was interrupted"],
    ["uncertain", "The run may still be active"],
  ])("surfaces %s after reload", (status, message) => {
    const history = { runs: [{ status }] } as History
    expect(runOutcome(history, true)).toContain(message)
  })
})

describe("question receipts", () => {
  it("does not offer a new submission after a write without confirmation", async () => {
    const { questionReceipt } =
      await import("../../src/features/conversations/state")
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
