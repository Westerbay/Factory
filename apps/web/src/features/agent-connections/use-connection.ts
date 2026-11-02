import { useEffect, useMemo, useSyncExternalStore } from "react"
import { useSession } from "../../lib/auth/session-context"
import { changeConnection, readConnection } from "./api"
import { ConnectionController } from "./connection-controller"
import { useModelCacheSync } from "./model-cache"

export function useConnection(available: boolean) {
  const { controller: session, state: sessionState } = useSession()
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
  useModelCacheSync(
    sessionState.status === "active" ? sessionState.session.user.id : null,
    "codex",
    state.connection?.status
  )
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
