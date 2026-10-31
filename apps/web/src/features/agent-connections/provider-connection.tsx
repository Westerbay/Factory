import { useId, useRef } from "react"
import { ArrowRight, CodeXml } from "lucide-react"

import { Button } from "../../components/ui/button"
import { connectionProblem } from "./api"
import type {
  ConnectionController,
  ConnectionState,
} from "./connection-controller"
import { DeviceCode } from "./device-code"
import "./connections.css"

export function ProviderConnection({
  state,
  controller,
}: {
  state: ConnectionState
  controller: ConnectionController
}) {
  const headingId = useId()
  const statusElement = useRef<HTMLParagraphElement>(null)
  const connection = state.connection
  const challenge = connection?.challenge
  const busy = state.pending !== null && state.pending !== "read"
  const canLogin =
    connection?.status === "disconnected" || connection?.status === "error"
  const issue =
    state.message ??
    connectionProblem(connection?.errorCode) ??
    (connection?.status === "unavailable"
      ? "Codex is unavailable. Check again to retry."
      : connection?.status === "error"
        ? "The account status is uncertain. Check again before connecting."
        : null)
  const checking = !connection && !state.message && !busy
  const status =
    state.pending === "login"
      ? "Starting sign-in…"
      : state.pending === "cancel"
        ? "Cancelling sign-in…"
        : state.pending === "logout"
          ? "Disconnecting Codex…"
          : challenge
            ? "Waiting for authorization…"
            : checking
              ? "Checking your provider…"
              : connection?.status === "connected"
                ? "Connected"
                : ""

  function act(action: () => Promise<void>) {
    // Mutations replace their trigger; polling must never move focus.
    statusElement.current?.focus()
    void action()
  }

  return (
    <section className="provider-connection" aria-labelledby={headingId}>
      <h1 id={headingId}>
        {challenge || busy ? "Connect Codex" : "Connect a provider"}
      </h1>
      <p
        ref={statusElement}
        tabIndex={-1}
        className="provider-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {status}
      </p>
      {canLogin && (
        <Button
          variant="secondary"
          className="provider-choice"
          disabled={busy || state.paused}
          onClick={() => act(controller.login)}
        >
          <CodeXml aria-hidden="true" />
          <span>Codex</span>
          <ArrowRight aria-hidden="true" />
        </Button>
      )}
      {challenge && (
        <DeviceCode key={challenge.operationId} challenge={challenge} />
      )}
      {issue && (
        <div className="provider-issue">
          <p role="status">{issue}</p>
          <Button
            variant="secondary"
            disabled={state.pending !== null || state.paused}
            onClick={() => act(controller.refresh)}
          >
            {state.pending === "read" ? "Checking…" : "Check again"}
          </Button>
        </div>
      )}
      {challenge && (
        <Button
          variant="ghost"
          className="provider-cancel"
          disabled={busy || state.paused}
          onClick={() => act(controller.cancelLogin)}
        >
          Cancel
        </Button>
      )}
    </section>
  )
}
