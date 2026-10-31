import { queryOptions } from "@tanstack/react-query"

import { models } from "./api"

export const modelQueryKey = (userId: string) =>
  ["codex-models", userId] as const

export function modelQueryOptions(userId: string) {
  return queryOptions({
    queryKey: modelQueryKey(userId),
    queryFn: ({ signal }) => models(signal),
    staleTime: 5 * 60_000,
    gcTime: 5 * 60_000,
    retry: false,
  })
}
