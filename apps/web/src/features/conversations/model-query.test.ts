import { QueryClient } from "@tanstack/react-query"
import { afterEach, expect, it, vi } from "vitest"

import { modelQueryOptions } from "./model-query"
import * as api from "./api"

afterEach(() => vi.restoreAllMocks())

it("reuses a user's fresh model list but isolates users and account invalidation", async () => {
  const client = new QueryClient()
  const read = vi
    .spyOn(api, "models")
    .mockResolvedValue([{ id: "model", name: "Model" }])
  const first = modelQueryOptions("user-one")
  const second = modelQueryOptions("user-two")
  await client.fetchQuery(first)
  await client.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(1)
  await client.fetchQuery(second)
  expect(read).toHaveBeenCalledTimes(2)
  await client.invalidateQueries({ queryKey: first.queryKey })
  await client.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(3)
  client.removeQueries({ queryKey: ["codex-models"] })
  await client.fetchQuery(first)
  expect(read).toHaveBeenCalledTimes(4)
  client.clear()
})
