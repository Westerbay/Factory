import { queryOptions } from "@tanstack/react-query"

import { ApiFailure } from "../../lib/api/client"
import { models } from "./api"
import { presentModels } from "./model-presentation"

const conflictRetryDelays = [2_000, 4_000, 8_000, 16_000, 30_000]

export const modelQueryKey = (userId: string) =>
  ["provider-models", userId] as const

export function modelQueryOptions(userId: string) {
  return queryOptions({
    queryKey: modelQueryKey(userId),
    queryFn: ({ signal }) => models(signal),
    staleTime: Infinity,
    select: presentModels,
    // Private session/account changes explicitly evict these queries. Navigation
    // should keep the last usable list even when the next discovery is busy.
    gcTime: Infinity,
    // Keep one recovery sequence, slowing to one attempt/minute after a minute.
    // TanStack pauses retries offline/in hidden tabs; the chat cancels on loss
    // of workspace availability. Other failures remain actionable immediately.
    retry: (_attempt, error) =>
      error instanceof ApiFailure && error.status === 409,
    retryDelay: (attempt) => conflictRetryDelays[attempt] ?? 60_000,
  })
}
