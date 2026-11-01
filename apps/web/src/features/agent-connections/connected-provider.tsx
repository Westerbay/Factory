import { useTranslation } from "react-i18next"
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
  const { t: translate } = useTranslation()
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
      ? translate("Disconnecting Codex…")
      : state.pending === "read" && !connection
        ? translate("Checking the account…")
        : (issue ??
          (connection?.status === "connected"
            ? translate("Connected")
            : connection?.status === "disconnected"
              ? translate("Not connected")
              : connection?.status === "connecting"
                ? translate("Waiting for authorization")
                : translate("The account status could not be confirmed.")))

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
        {translate(status)}
      </p>
      {connection?.account && (
        <dl className="provider-account">
          <div>
            <dt>{translate("Account")}</dt>
            <dd>
              {connection.account.email ??
                translate("Personal ChatGPT account")}
            </dd>
          </div>
          <div>
            <dt>{translate("Plan")}</dt>
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
          {state.pending === "read"
            ? translate("Checking…")
            : translate("Check again")}
        </Button>
      )}
      {canDisconnect && (
        <div className="provider-disconnect">
          {confirming ? (
            <div className="provider-disconnect-confirmation">
              <p>
                {translate(
                  "Disconnect Codex? This removes its saved sign-in and keeps your files. Terminal processes may continue; Stop environment ends all running processes."
                )}{" "}
              </p>
              <div className="provider-actions">
                <Button
                  ref={cancelButton}
                  variant="secondary"
                  onClick={cancelConfirmation}
                  disabled={busy || state.paused}
                >
                  {translate("Cancel")}{" "}
                </Button>
                <Button
                  variant="danger"
                  onClick={disconnect}
                  disabled={busy || state.paused}
                >
                  {translate("Disconnect Codex")}{" "}
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
              {translate("Disconnect Codex")}{" "}
            </Button>
          )}
        </div>
      )}
    </section>
  )
}
