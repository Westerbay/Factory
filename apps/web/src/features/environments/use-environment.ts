import { useEffect, useMemo, useSyncExternalStore } from "react"

import { useSession } from "../../lib/auth/session-context"
import { changeEnvironment, readEnvironment } from "./api"
import { EnvironmentController } from "./environment-controller"

/** One lifecycle subscription shared by the workspace's environment consumers. */
export function useEnvironment() {
  const { controller: session } = useSession()
  const controller = useMemo(
    () =>
      new EnvironmentController({
        session,
        visible: () => document.visibilityState === "visible",
        online: () => navigator.onLine,
        read: readEnvironment,
        change: changeEnvironment,
      }),
    [session]
  )
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
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
