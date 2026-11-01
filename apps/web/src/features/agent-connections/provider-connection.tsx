import { useTranslation } from "react-i18next"
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
  onClaude,
  claudeAvailable,
  inSettings = false,
}: {
  state: ConnectionState
  controller: ConnectionController
  onClaude?: () => void
  claudeAvailable?: boolean
  inSettings?: boolean
}) {
  const { t: translate } = useTranslation()
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
      ? translate("Codex is unavailable. Check again to retry.")
      : connection?.status === "error"
        ? translate(
            "The account status is uncertain. Check again before connecting."
          )
        : null)
  const checking = !connection && !state.message && !busy
  const Heading = inSettings ? "h3" : "h1"
  const status =
    state.pending === "login"
      ? translate("Starting sign-in…")
      : state.pending === "cancel"
        ? translate("Cancelling sign-in…")
        : state.pending === "logout"
          ? translate("Disconnecting Codex…")
          : challenge
            ? translate("Waiting for authorization…")
            : checking
              ? translate("Checking your provider…")
              : connection?.status === "connected"
                ? translate("Connected")
                : ""

  function act(action: () => Promise<void>) {
    // Mutations replace their trigger; polling must never move focus.
    statusElement.current?.focus()
    void action()
  }

  return (
    <section className="provider-connection" aria-labelledby={headingId}>
      <Heading id={headingId}>
        {inSettings
          ? "Codex"
          : challenge || busy
            ? translate("Connect Codex")
            : translate("Connect a provider")}
      </Heading>
      <p
        ref={statusElement}
        tabIndex={-1}
        className="provider-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {translate(status)}
      </p>
      {canLogin && (
        <Button
          variant="secondary"
          className="provider-choice"
          disabled={busy || state.paused}
          onClick={() => act(controller.login)}
        >
          <CodeXml aria-hidden="true" />
          <span>{inSettings ? translate("Connect Codex") : "Codex"}</span>
          <ArrowRight aria-hidden="true" />
        </Button>
      )}
      {onClaude && !challenge && !busy && (
        <Button
          variant="secondary"
          className="provider-choice"
          disabled={!claudeAvailable}
          onClick={onClaude}
        >
          <CodeXml aria-hidden="true" />
          <span>Claude Code</span>
          <ArrowRight aria-hidden="true" />
        </Button>
      )}
      {challenge && (
        <DeviceCode key={challenge.operationId} challenge={challenge} />
      )}
      {issue && (
        <div className="provider-issue">
          <p role="status">{translate(issue)}</p>
          <Button
            variant="secondary"
            disabled={state.pending !== null || state.paused}
            onClick={() => act(controller.refresh)}
          >
            {state.pending === "read"
              ? translate("Checking…")
              : translate("Check again")}
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
          {translate("Cancel")}{" "}
        </Button>
      )}
    </section>
  )
}
