import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
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
  useLocale()
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
      ? m.codex_is_unavailable_check_again_to_retry()
      : connection?.status === "error"
        ? m.the_account_status_is_uncertain_check_again_before_connecting()
        : null)
  const checking = !connection && !state.message && !busy
  const Heading = inSettings ? "h3" : "h1"
  const status =
    state.pending === "login"
      ? m.starting_sign_in()
      : state.pending === "cancel"
        ? m.cancelling_sign_in()
        : state.pending === "logout"
          ? m.disconnecting_codex()
          : challenge
            ? m.waiting_for_authorization()
            : checking
              ? m.checking_your_provider()
              : connection?.status === "connected"
                ? m.connected()
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
            ? m.connect_codex()
            : m.connect_a_provider()}
      </Heading>
      <p
        ref={statusElement}
        tabIndex={-1}
        className="provider-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {translateMessage(status)}
      </p>
      {canLogin && (
        <Button
          variant="secondary"
          className="provider-choice"
          disabled={busy || state.paused}
          onClick={() => act(controller.login)}
        >
          <CodeXml aria-hidden="true" />
          <span>{inSettings ? m.connect_codex() : "Codex"}</span>
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
          <p role="status">{translateMessage(issue)}</p>
          <Button
            variant="secondary"
            disabled={state.pending !== null || state.paused}
            onClick={() => act(controller.refresh)}
          >
            {state.pending === "read" ? m.checking() : m.check_again()}
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
          {m.cancel()}{" "}
        </Button>
      )}
    </section>
  )
}
