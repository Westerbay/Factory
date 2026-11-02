import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useEffect, useId, useRef, useState } from "react"

import { Button } from "../../components/ui/button"
import { canDisconnectConnection, connectionProblem } from "./api"
import type {
  ConnectionController,
  ConnectionState,
} from "./connection-controller"
import "./connections.css"

export function ConnectedProvider({
  state,
  controller,
}: {
  state: ConnectionState
  controller: ConnectionController
}) {
  useLocale()
  const headingId = useId()
  const statusElement = useRef<HTMLParagraphElement>(null)
  const disconnectButton = useRef<HTMLButtonElement>(null)
  const cancelButton = useRef<HTMLButtonElement>(null)
  const wasConfirming = useRef(false)
  const [confirming, setConfirming] = useState(false)
  const connection = state.connection
  const busy = state.pending !== null && state.pending !== "read"
  const canDisconnect = canDisconnectConnection(connection)
  const issue = state.message ?? connectionProblem(connection?.errorCode)
  const needsCheck =
    Boolean(issue) ||
    connection?.status === "error" ||
    connection?.status === "unavailable"
  const status =
    state.pending === "logout"
      ? m.disconnecting_codex()
      : state.pending === "read" && !connection
        ? m.checking_the_account()
        : (issue ??
          (connection?.status === "connected"
            ? m.connected()
            : connection?.status === "disconnected"
              ? m.not_connected()
              : connection?.status === "connecting"
                ? m.waiting_for_authorization_738e149b()
                : m.the_account_status_could_not_be_confirmed()))

  useEffect(() => {
    if (confirming) cancelButton.current?.focus()
    else if (wasConfirming.current) disconnectButton.current?.focus()
    wasConfirming.current = confirming
  }, [confirming])

  function showConfirmation() {
    setConfirming(true)
  }
  function cancelConfirmation() {
    setConfirming(false)
  }
  function disconnect() {
    setConfirming(false)
    statusElement.current?.focus()
    void controller.logout()
  }
  function check() {
    statusElement.current?.focus()
    void controller.refresh()
  }

  return (
    <section className="provider-settings" aria-labelledby={headingId}>
      <h3 id={headingId}>Codex</h3>
      <p
        ref={statusElement}
        tabIndex={-1}
        className="provider-settings-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {translateMessage(status)}
      </p>
      {connection?.account && (
        <dl className="provider-account">
          <div>
            <dt>{m.account()}</dt>
            <dd>{connection.account.email ?? m.personal_chatgpt_account()}</dd>
          </div>
          <div>
            <dt>{m.plan()}</dt>
            <dd>{connection.account.plan}</dd>
          </div>
        </dl>
      )}
      {needsCheck && (
        <Button
          variant="secondary"
          onClick={check}
          disabled={state.pending !== null || state.paused}
        >
          {state.pending === "read" ? m.checking() : m.check_again()}
        </Button>
      )}
      {canDisconnect && (
        <div className="provider-disconnect">
          {confirming ? (
            <div className="provider-disconnect-confirmation">
              <p>
                {m.disconnect_codex_this_removes_its_saved_sign_in_and_keeps_your_fi()}{" "}
              </p>
              <div className="provider-actions">
                <Button
                  ref={cancelButton}
                  variant="secondary"
                  onClick={cancelConfirmation}
                  disabled={busy || state.paused}
                >
                  {m.cancel()}{" "}
                </Button>
                <Button
                  variant="danger"
                  onClick={disconnect}
                  disabled={busy || state.paused}
                >
                  {m.disconnect_codex()}{" "}
                </Button>
              </div>
            </div>
          ) : (
            <Button
              ref={disconnectButton}
              variant="ghost"
              onClick={showConfirmation}
              disabled={busy || state.paused}
            >
              {m.disconnect_codex()}{" "}
            </Button>
          )}
        </div>
      )}
    </section>
  )
}
