import { useEffect, useRef } from "react"
import { useQueryClient, type QueryClient } from "@tanstack/react-query"

import type { Model } from "../conversations/api"
import { modelQueryKey } from "../conversations/model-query"

type ObservedConnection = {
  userId: string
  status: "connected" | "disconnected"
}

/** Invalidate only when the provider's confirmed membership changes. */
export function observeModelConnection(
  client: QueryClient,
  userId: string | null,
  provider: "codex" | "claude",
  status: string | undefined,
  previous: ObservedConnection | undefined
): ObservedConnection | undefined {
  if (!userId) return undefined
  if (status !== "connected" && status !== "disconnected")
    return previous?.userId === userId ? previous : undefined

  const cached = client.getQueryData<Model[]>(modelQueryKey(userId))
  const listed = cached?.some((model) =>
    model.id.startsWith(`factory:${provider}:`)
  )
  const changed = previous?.userId === userId && previous.status !== status
  const inconsistent =
    listed !== undefined && listed !== (status === "connected")
  if (changed || inconsistent)
    void client.resetQueries({ queryKey: modelQueryKey(userId), exact: true })
  return { userId, status }
}

export function useModelCacheSync(
  userId: string | null,
  provider: "codex" | "claude",
  status: string | undefined
) {
  const client = useQueryClient()
  const previous = useRef<ObservedConnection | undefined>(undefined)
  useEffect(() => {
    previous.current = observeModelConnection(
      client,
      userId,
      provider,
      status,
      previous.current
    )
  }, [client, userId, provider, status])
}
