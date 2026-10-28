import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
  type ReactNode,
} from "react"
import { useQueryClient } from "@tanstack/react-query"

import { onAuthorizationFailure } from "../api/client"
import { readSession, revokeSession } from "./session-api"
import { SessionController } from "./session-controller"

const SessionContext = createContext<SessionController | null>(null)

export function SessionProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient()
  const channel = useRef<BroadcastChannel | null>(null)
  const controller = useMemo(
    () =>
      new SessionController({
        read: readSession,
        revoke: revokeSession,
        clearPrivateData: () => queryClient.clear(),
        announceChange: () => channel.current?.postMessage("invalidate"),
        online: () => navigator.onLine,
      }),
    [queryClient]
  )

  useEffect(() => {
    function refresh() {
      void controller.refresh()
    }
    function offline() {
      controller.invalidate("offline")
    }
    function changed(event: MessageEvent) {
      if (event.data !== "invalidate") return
      void controller.invalidateRemote()
    }
    if ("BroadcastChannel" in window) {
      channel.current = new BroadcastChannel("factory-session")
      channel.current.addEventListener("message", changed)
    }
    const unsubscribe = onAuthorizationFailure(() =>
      controller.authorizationFailed()
    )
    window.addEventListener("focus", refresh)
    window.addEventListener("online", refresh)
    window.addEventListener("offline", offline)
    refresh()
    return () => {
      unsubscribe()
      window.removeEventListener("focus", refresh)
      window.removeEventListener("online", refresh)
      window.removeEventListener("offline", offline)
      channel.current?.close()
      channel.current = null
      controller.dispose()
    }
  }, [controller])

  return (
    <SessionContext.Provider value={controller}>
      {children}
    </SessionContext.Provider>
  )
}

export function useSession() {
  const controller = useContext(SessionContext)
  if (!controller) throw new Error("SessionProvider is required")
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
  )
  return { state, controller }
}
