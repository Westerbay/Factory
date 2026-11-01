import { useEffect, useMemo, useSyncExternalStore } from "react"
import { useQueryClient } from "@tanstack/react-query"
import { useSession } from "../../lib/auth/session-context"
import { changeConnection, readConnection } from "./api"
import { ConnectionController } from "./connection-controller"

export function useConnection(available: boolean) {
  const { controller: session } = useSession()
  const queryClient = useQueryClient()
  const controller = useMemo(
    () =>
      new ConnectionController({
        session,
        visible: () => document.visibilityState === "visible",
        online: () => navigator.onLine,
        available: () => available,
        read: readConnection,
        change: changeConnection,
      }),
    [session, available]
  )
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
  )
  useEffect(() => {
    if (
      state.pending === "login" ||
      state.pending === "logout" ||
      state.connection?.status === "disconnected"
    ) {
      void queryClient.resetQueries({ queryKey: ["provider-models"] })
    }
  }, [queryClient, state.pending, state.connection?.status])
  useEffect(() => {
    controller.connect()
    document.addEventListener("visibilitychange", controller.synchronize)
    window.addEventListener("online", controller.synchronize)
    window.addEventListener("offline", controller.synchronize)
    return () => {
      document.removeEventListener("visibilitychange", controller.synchronize)
      window.removeEventListener("online", controller.synchronize)
      window.removeEventListener("offline", controller.synchronize)
      controller.dispose()
    }
  }, [controller])
  return { state, controller }
}
