import {
  environmentProblem,
  environmentUpgradeMessage,
  type Environment,
} from "./api"
import {
  canStopEnvironment,
  canStartEnvironment,
  type EnvironmentController,
  type EnvironmentState,
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
    "Your environment is running. It keeps running when you close this page or sign out.",
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

export function EnvironmentCard({
  state,
  controller,
}: {
  state: EnvironmentState
  controller: EnvironmentController
}) {
  const status = state.environment?.status
  const busy = state.pending === "start" || state.pending === "stop"
  const canStart = canStartEnvironment(state.environment)
  const upgradeRequired = state.environment?.upgradeRequired
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
            {upgradeRequired ? `Upgrade required — ${label}` : label}
            {state.pending === "start" ? " — Requesting start" : ""}
            {state.pending === "stop" ? " — Requesting stop" : ""}
          </p>
          <p>
            {upgradeRequired
              ? environmentUpgradeMessage
              : status
                ? statusHelp[status]
                : "Waiting for a confirmed environment status."}
          </p>
        </div>
        <dl className="environment-dates">
          <div>
            <dt>Last observed</dt>
            <dd>{timestamp(state.environment?.observedAt)}</dd>
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
        Closing this page or signing out does not stop your environment. Stop it
        explicitly when you are finished. An administrator can stop it by
        suspending your account. Saved files are kept.
      </p>
    </section>
  )
}
