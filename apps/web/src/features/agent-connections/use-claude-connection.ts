import { useEffect, useRef } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"

import { useSession } from "../../lib/auth/session-context"
import { readClaude } from "./claude-api"

export function useClaudeConnection(available: boolean, polling: boolean) {
  const { state } = useSession()
  const queryClient = useQueryClient()
  const userId = state.status === "active" ? state.session.user.id : null
  const previous = useRef<string | undefined>(undefined)
  const query = useQuery({
    queryKey: ["claude-connection", userId],
    queryFn: ({ signal }) => readClaude(signal),
    enabled: available && userId !== null,
    refetchInterval: polling ? 2_000 : false,
    refetchIntervalInBackground: false,
    retry: false,
    staleTime: 5_000,
  })
  useEffect(() => {
    const status = query.data?.status
    if (status !== undefined && previous.current !== status) {
      void queryClient.resetQueries({ queryKey: ["provider-models"] })
    }
    previous.current = status
  }, [queryClient, query.data?.status])
  return query
}
