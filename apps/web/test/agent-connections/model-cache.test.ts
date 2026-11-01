import { QueryClient } from "@tanstack/react-query"
import { expect, it, vi } from "vitest"

import { observeModelConnection } from "../../src/features/agent-connections/model-cache"
import { modelQueryKey } from "../../src/features/conversations/model-query"

it("retains cached models for unchanged and transient provider observations", () => {
  const client = new QueryClient()
  const key = modelQueryKey("user-one")
  const models = [
    {
      id: "factory:claude:opus",
      name: "Claude · Opus",
      effortLevels: [],
      resolvedModel: null,
      defaultEffort: null,
    },
  ]
  client.setQueryData(key, models)
  const reset = vi.spyOn(client, "resetQueries")
  const codex = observeModelConnection(
    client,
    "user-one",
    "codex",
    "disconnected",
    undefined
  )
  const claude = observeModelConnection(
    client,
    "user-one",
    "claude",
    "connected",
    undefined
  )
  expect(
    observeModelConnection(client, "user-one", "codex", "unavailable", codex)
  ).toEqual(codex)
  expect(
    observeModelConnection(client, "user-one", "claude", "connected", claude)
  ).toEqual(claude)
  expect(reset).not.toHaveBeenCalled()
  expect(client.getQueryData(key)).toEqual(models)
  client.clear()
})

it("clears only the affected user's models when a provider connects or disconnects", () => {
  const client = new QueryClient()
  const one = modelQueryKey("user-one")
  const two = modelQueryKey("user-two")
  const models = [
    {
      id: "factory:claude:opus",
      name: "Claude · Opus",
      effortLevels: [],
      resolvedModel: null,
      defaultEffort: null,
    },
  ]
  client.setQueryData(one, models)
  client.setQueryData(two, models)
  const initial = observeModelConnection(
    client,
    "user-one",
    "codex",
    "disconnected",
    undefined
  )
  observeModelConnection(client, "user-one", "codex", "connected", initial)
  expect(client.getQueryData(one)).toBeUndefined()
  expect(client.getQueryData(two)).toEqual(models)
  client.setQueryData(one, models)
  observeModelConnection(
    client,
    "user-one",
    "claude",
    "disconnected",
    undefined
  )
  expect(client.getQueryData(one)).toBeUndefined()
  client.clear()
})

it("cancels an in-flight discovery when a provider connects", async () => {
  const client = new QueryClient()
  const key = modelQueryKey("user-one")
  let finish!: (value: unknown[]) => void
  let signal: AbortSignal | undefined
  const pending = client
    .fetchQuery({
      queryKey: key,
      queryFn: ({ signal: requestSignal }) => {
        signal = requestSignal
        return new Promise<unknown[]>((resolve) => {
          finish = resolve
        })
      },
    })
    .catch(() => undefined)
  const previous = observeModelConnection(
    client,
    "user-one",
    "claude",
    "disconnected",
    undefined
  )
  observeModelConnection(client, "user-one", "claude", "connected", previous)
  expect(signal?.aborted).toBe(true)
  finish([{ id: "outdated" }])
  await pending
  expect(client.getQueryData(key)).toBeUndefined()
  client.clear()
})
