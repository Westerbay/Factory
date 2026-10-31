import { useEffect, useMemo, useSyncExternalStore } from "react"

import { useSession } from "../../lib/auth/session-context"
import type { SessionController } from "../../lib/auth/session-controller"
import { changeEnvironment, readEnvironment } from "./api"
import { EnvironmentController } from "./environment-controller"

// Private routes can remount on navigation or access loss. Retain this in-memory
// lifecycle owner so re-entering the same session cannot undo Stop.
// dispose() still aborts requests and clears displayed private environment data.
const controllers = new WeakMap<SessionController, EnvironmentController>()

function environmentFor(
  session: SessionController,
  hasRecoverableRuns: (signal: AbortSignal) => Promise<boolean>
) {
  const existing = controllers.get(session)
  if (existing) return existing
  const controller = new EnvironmentController({
    session,
    visible: () => document.visibilityState === "visible",
    online: () => navigator.onLine,
    read: readEnvironment,
    change: changeEnvironment,
    hasRecoverableRuns: (signal) =>
      hasRecoverableRuns(
        AbortSignal.any([signal, AbortSignal.timeout(10_000)])
      ),
  })
  controllers.set(session, controller)
  return controller
}

/** One lifecycle subscription shared by the workspace's environment consumers. */
export function useEnvironment(
  hasRecoverableRuns: (signal: AbortSignal) => Promise<boolean>
) {
  const { controller: session } = useSession()
  const controller = useMemo(
    () => environmentFor(session, hasRecoverableRuns),
    [session, hasRecoverableRuns]
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
