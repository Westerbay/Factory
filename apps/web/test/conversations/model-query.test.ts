import {
  environmentManager,
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query"
import { afterEach, beforeEach, expect, it, vi } from "vitest"

import { ApiFailure } from "../../src/lib/api/client"
import { modelQueryOptions } from "../../src/features/conversations/model-query"
import * as api from "../../src/features/conversations/api"

const models = [
  {
    id: "model",
    name: "Model",
    effortLevels: [],
    resolvedModel: null,
    defaultEffort: null,
  },
]
const clients: QueryClient[] = []
beforeEach(() => {
  // Exercise QueryObserver's browser lifecycle without a DOM.
  vi.spyOn(environmentManager, "isServer").mockReturnValue(false)
})
function client() {
  const value = new QueryClient()
  clients.push(value)
  return value
}

function deferredModels() {
  let resolve!: (value: api.Model[]) => void
  const promise = new Promise<api.Model[]>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

afterEach(() => {
  for (const value of clients.splice(0)) {
    value.unmount()
    value.clear()
  }
  focusManager.setFocused(undefined)
  vi.useRealTimers()
  vi.restoreAllMocks()
})

it("reuses a user's fresh model list but isolates users and account invalidation", async () => {
  const cache = client()
  const read = vi.spyOn(api, "models").mockResolvedValue(models)
  const first = modelQueryOptions("user-one")
  const second = modelQueryOptions("user-two")
  await cache.fetchQuery(first)
  await cache.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(1)
  await cache.fetchQuery(second)
  expect(read).toHaveBeenCalledTimes(2)
  await cache.invalidateQueries({ queryKey: first.queryKey })
  await cache.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(3)
  cache.removeQueries({ queryKey: ["provider-models"] })
  await cache.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(4)
})

it("shares a pending discovery across consumers and focus refetches", async () => {
  const cache = client()
  cache.mount()
  const response = deferredModels()
  const read = vi.spyOn(api, "models").mockReturnValue(response.promise)
  const options = modelQueryOptions("user-one")
  const observer = new QueryObserver(cache, options)
  const unsubscribe = observer.subscribe(() => {})
  const first = cache.fetchQuery(options)
  const second = cache.fetchQuery(options)
  focusManager.setFocused(false)
  focusManager.setFocused(true)
  expect(read).toHaveBeenCalledTimes(1)
  response.resolve(models)
  expect(await first).toEqual(models)
  expect(await second).toEqual(models)
  unsubscribe()
})

it("recovers transient conflicts automatically without entering the final error state", async () => {
  vi.useFakeTimers()
  const cache = client()
  const read = vi
    .spyOn(api, "models")
    .mockRejectedValueOnce(new ApiFailure(409))
    .mockRejectedValueOnce(new ApiFailure(409))
    .mockResolvedValue(models)
  const observer = new QueryObserver(cache, modelQueryOptions("user-one"))
  const states: string[] = []
  const unsubscribe = observer.subscribe((state) => states.push(state.status))
  await vi.advanceTimersByTimeAsync(1_999)
  expect(read).toHaveBeenCalledTimes(1)
  await vi.advanceTimersByTimeAsync(1)
  expect(read).toHaveBeenCalledTimes(2)
  await vi.advanceTimersByTimeAsync(4_000)
  expect(observer.getCurrentResult().data).toEqual(models)
  expect(states).not.toContain("error")
  unsubscribe()
})

it("keeps the last valid list and slows persistent conflicts to one attempt per minute", async () => {
  vi.useFakeTimers()
  const cache = client()
  const options = modelQueryOptions("user-one")
  cache.setQueryData(options.queryKey, models)
  const read = vi.spyOn(api, "models").mockRejectedValue(new ApiFailure(409))
  const observer = new QueryObserver(cache, options)
  const unsubscribe = observer.subscribe(() => {})
  void cache.invalidateQueries(
    { queryKey: options.queryKey },
    { cancelRefetch: false }
  )
  await vi.advanceTimersByTimeAsync(60_000)
  expect(read).toHaveBeenCalledTimes(6)
  expect(observer.getCurrentResult().isError).toBe(false)
  expect(observer.getCurrentResult().failureCount).toBe(6)
  expect(observer.getCurrentResult().data).toEqual(models)
  await vi.advanceTimersByTimeAsync(60_000)
  expect(read).toHaveBeenCalledTimes(7)
  await vi.advanceTimersByTimeAsync(59_999)
  expect(read).toHaveBeenCalledTimes(7)
  read.mockResolvedValue(models)
  await vi.advanceTimersByTimeAsync(1)
  expect(read).toHaveBeenCalledTimes(8)
  expect(observer.getCurrentResult().isSuccess).toBe(true)
  unsubscribe()
})

it("keeps the session's model list across focus changes and long navigation gaps", async () => {
  vi.useFakeTimers()
  const cache = client()
  cache.mount()
  const options = modelQueryOptions("user-one")
  const read = vi.spyOn(api, "models").mockResolvedValue(models)
  await cache.fetchQuery(options)
  const observer = new QueryObserver(cache, options)
  const unsubscribe = observer.subscribe(() => {})
  await vi.advanceTimersByTimeAsync(30 * 60_000)
  focusManager.setFocused(false)
  focusManager.setFocused(true)
  await vi.advanceTimersByTimeAsync(0)
  unsubscribe()
  const reconnected = new QueryObserver(cache, options)
  const stop = reconnected.subscribe(() => {})
  await vi.advanceTimersByTimeAsync(0)
  expect(read).toHaveBeenCalledTimes(1)
  expect(cache.getQueryData(options.queryKey)).toEqual(models)
  read.mockRejectedValue(new ApiFailure(503))
  await cache.invalidateQueries({
    queryKey: options.queryKey,
    refetchType: "none",
  })
  await expect(cache.fetchQuery(options)).rejects.toMatchObject({ status: 503 })
  expect(cache.getQueryData(options.queryKey)).toEqual(models)
  stop()
})

it.each([401, 403, 429, 503])(
  "does not treat HTTP %i as a transient conflict",
  async (status) => {
    const cache = client()
    const read = vi
      .spyOn(api, "models")
      .mockRejectedValue(new ApiFailure(status))
    await expect(
      cache.fetchQuery(modelQueryOptions("user-one"))
    ).rejects.toMatchObject({ status })
    expect(read).toHaveBeenCalledTimes(1)
  }
)

it("cancels conflict backoff when unavailable and can resume with the cached list", async () => {
  vi.useFakeTimers()
  const cache = client()
  const options = modelQueryOptions("user-one")
  cache.setQueryData(options.queryKey, models)
  const read = vi.spyOn(api, "models").mockRejectedValue(new ApiFailure(409))
  const observer = new QueryObserver(cache, options)
  const unsubscribe = observer.subscribe(() => {})
  void cache.invalidateQueries(
    { queryKey: options.queryKey },
    { cancelRefetch: false }
  )
  await vi.advanceTimersByTimeAsync(0)
  observer.setOptions({ ...options, enabled: false })
  await cache.cancelQueries({ queryKey: options.queryKey })
  await vi.advanceTimersByTimeAsync(120_000)
  expect(read).toHaveBeenCalledTimes(1)
  expect(observer.getCurrentResult().data).toEqual(models)
  read.mockResolvedValue(models)
  observer.setOptions(options)
  await vi.advanceTimersByTimeAsync(0)
  expect(read).toHaveBeenCalledTimes(2)
  expect(observer.getCurrentResult().isSuccess).toBe(true)
  unsubscribe()
})

it("aborts invalidated private reads and cannot repopulate cleared account data", async () => {
  const cache = client()
  const options = modelQueryOptions("user-one")
  const response = deferredModels()
  let signal: AbortSignal | undefined
  vi.spyOn(api, "models").mockImplementation((requestSignal) => {
    signal = requestSignal
    return response.promise
  })
  const pending = cache.fetchQuery(options).catch(() => undefined)
  cache.clear()
  expect(signal?.aborted).toBe(true)
  response.resolve(models)
  await pending
  expect(cache.getQueryData(options.queryKey)).toBeUndefined()
})
