import { useEffect, useRef, useState } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"

import { useSession } from "../../lib/auth/session-context"
import {
  beginClaude,
  cancelClaude,
  readClaude,
  submitClaudeCode,
} from "./claude-api"
import { useModelCacheSync } from "./model-cache"

export function useClaudeConnection(available: boolean, polling: boolean) {
  const { state } = useSession()
  const queryClient = useQueryClient()
  const session = state.status === "active" ? state.session : null
  const userId = session?.user.id ?? null
  const currentSession = useRef(session)
  currentSession.current = session
  const busy = useRef(false)
  const [pending, setPending] = useState<"login" | "cancel" | "code" | null>(
    null
  )
  const [message, setMessage] = useState<string | null>(null)
  const [submitted, setSubmitted] = useState<string | null>(null)
  const query = useQuery({
    queryKey: ["claude-connection", userId],
    queryFn: ({ signal }) => readClaude(signal),
    enabled: available && userId !== null,
    refetchInterval: (query) =>
      polling || query.state.data?.status === "connecting" ? 2_000 : false,
    refetchIntervalInBackground: false,
    retry: false,
    staleTime: 5_000,
  })
  useModelCacheSync(userId, "claude", query.data?.status)
  useEffect(() => {
    setSubmitted(null)
    setMessage(null)
  }, [userId])
  useEffect(() => {
    const challenge = query.data?.challenge
    if (!challenge || !userId) return
    const expiresIn = Math.max(0, Date.parse(challenge.expiresAt) - Date.now())
    const timer = setTimeout(
      () => {
        queryClient.setQueryData(
          ["claude-connection", userId],
          (previous: typeof query.data) =>
            previous?.challenge?.operationId === challenge.operationId
              ? {
                  provider: "claude",
                  status: "error",
                  account: null,
                  challenge: null,
                  errorCode: "expired",
                }
              : previous
        )
      },
      Math.min(expiresIn, 2_147_483_647)
    )
    return () => clearTimeout(timer)
  }, [query.data?.challenge, queryClient, userId])

  async function run(kind: "login" | "cancel" | "code", code?: string) {
    if (!session || !available || busy.current) return false
    const sameSession = () =>
      currentSession.current?.user.id === session.user.id &&
      currentSession.current.csrfToken === session.csrfToken &&
      currentSession.current.user.role === session.user.role
    const operationId = query.data?.challenge?.operationId
    if (kind !== "login" && !operationId) return false
    busy.current = true
    setPending(kind)
    setMessage(null)
    if (kind === "code" && operationId) setSubmitted(operationId)
    try {
      await queryClient.cancelQueries({
        queryKey: ["claude-connection", userId],
      })
      let result
      if (kind === "login") result = await beginClaude(session.csrfToken)
      else if (!operationId) return false
      else if (kind === "cancel")
        result = await cancelClaude(session.csrfToken, operationId)
      else if (code !== undefined)
        result = await submitClaudeCode(session.csrfToken, operationId, code)
      else return false
      if (!sameSession()) return false
      await queryClient.cancelQueries({
        queryKey: ["claude-connection", userId],
      })
      if (!sameSession()) return false
      queryClient.setQueryData(["claude-connection", userId], result)
      if (kind === "login") setSubmitted(null)
      return true
    } catch {
      if (sameSession())
        setMessage(
          kind === "code"
            ? "The code could not be confirmed. Cancel and start again."
            : "Claude sign-in could not be updated. Check the connection and try again."
        )
      return false
    } finally {
      busy.current = false
      setPending(null)
    }
  }
  return {
    ...query,
    pending,
    message,
    submitted: submitted === query.data?.challenge?.operationId,
    begin: () => run("login"),
    cancel: () => run("cancel"),
    submit: (code: string) => run("code", code),
  }
}
