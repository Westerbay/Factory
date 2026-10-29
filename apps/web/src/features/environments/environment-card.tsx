import { useEffect, useMemo, useSyncExternalStore } from "react"

import { useSession } from "../../lib/auth/session-context"
import {
  changeEnvironment,
  environmentProblem,
  readEnvironment,
  type Environment,
} from "./api"
import {
  canStopEnvironment,
  EnvironmentController,
} from "./environment-controller"
import "./environments.css"

const statusLabels: Record<Environment["status"], string> = {
  not_created: "Not created",
  starting: "Starting",
  running: "Running",
  stopping: "Stopping",
  stopped: "Stopped",
  error: "Error",
  unavailable: "Unavailable",
}
const statusHelp: Record<Environment["status"], string> = {
  not_created: "Start your environment when you need it.",
  starting:
    "Your environment is starting. This page checks its status automatically.",
  running:
    "Your environment is running. Keeping this page visible renews its idle lease.",
  stopping: "Your environment is stopping. Your saved files are kept.",
  stopped:
    "Your environment is stopped. Start it again to resume with your saved files.",
  error:
    "The environment needs attention. Refresh its status or request a stop before trying again.",
  unavailable:
    "Environments are currently unavailable. Refresh to check again, or contact your administrator.",
}

function timestamp(value: string | null | undefined) {
  return value ? (
    <time dateTime={value}>{new Date(value).toLocaleString()}</time>
  ) : (
    "Not available"
  )
}

export function EnvironmentCard() {
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

  const status = state.environment?.status
  const busy = state.pending === "start" || state.pending === "stop"
  const canStart = status === "not_created" || status === "stopped"
  const canStop = canStopEnvironment(state.environment)
  const label = status
    ? statusLabels[status]
    : state.pending
      ? "Checking"
      : "Unknown"

  return (
    <section aria-labelledby="environment-heading">
      <p className="eyebrow">Workspace</p>
      <h1 id="environment-heading">Your environment</h1>
      <p className="introduction">
        Start your private environment when you need it. Stopping keeps your
        saved files.
      </p>
      <div className="connection-panel environment-card">
        <div className="environment-summary">
          <h2>Status</h2>
          <p
            className="environment-status"
            role="status"
            aria-live="polite"
            aria-atomic="true"
          >
            {label}
            {state.pending === "start" ? " — Requesting start" : ""}
            {state.pending === "stop" ? " — Requesting stop" : ""}
          </p>
          <p>
            {status
              ? statusHelp[status]
              : "Waiting for a confirmed environment status."}
          </p>
        </div>
        <dl className="environment-dates">
          <div>
            <dt>Last observed</dt>
            <dd>{timestamp(state.environment?.observedAt)}</dd>
          </div>
          <div>
            <dt>Idle lease expires</dt>
            <dd>{timestamp(state.environment?.idleExpiresAt)}</dd>
          </div>
        </dl>
        <p
          className="environment-feedback"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          {state.message ??
            environmentProblem(state.environment?.errorCode) ??
            (state.paused
              ? "Updates are paused while this page is hidden, offline or your session is being checked."
              : "")}
        </p>
        <div className="environment-actions" aria-label="Environment actions">
          <button
            type="button"
            onClick={controller.start}
            disabled={!canStart || busy || state.paused}
          >
            Start environment
          </button>
          <button
            type="button"
            onClick={controller.stop}
            disabled={!canStop || busy || state.paused}
          >
            Stop environment
          </button>
          <button
            type="button"
            onClick={controller.refresh}
            disabled={busy || state.paused}
          >
            Refresh
          </button>
        </div>
      </div>
      <p className="connection-help">
        Returning to this page renews an active, unexpired idle lease. While
        visible, this page renews it at least every minute, before it expires.
        Leaving this page or signing out stops renewal. Your environment stops
        after its lease expires while the service is available.
      </p>
    </section>
  )
}
