import { useTranslation } from "react-i18next"
import { useEffect, useId, useRef, useState } from "react"

import { Button } from "../../components/ui/button"
import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import { claudeConnectionProblem, logoutClaude } from "./claude-api"
import { useClaudeConnection } from "./use-claude-connection"

export function ClaudeSettings({
  connection,
  available,
  onConnect,
}: {
  connection: ReturnType<typeof useClaudeConnection>
  available: boolean
  onConnect: () => void
}) {
  const { t: translate } = useTranslation()
  const { state } = useSession()
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const headingId = useId()
  const statusElement = useRef<HTMLParagraphElement>(null)
  const disconnectButton = useRef<HTMLButtonElement>(null)
  const cancelButton = useRef<HTMLButtonElement>(null)
  const wasConfirming = useRef(false)
  const connected = connection.data?.status === "connected"
  useEffect(() => {
    if (confirming) cancelButton.current?.focus()
    else if (wasConfirming.current) disconnectButton.current?.focus()
    wasConfirming.current = confirming
  }, [confirming])
  async function disconnect() {
    if (state.status !== "active") return
    setPending(true)
    setError(null)
    statusElement.current?.focus()
    try {
      await logoutClaude(state.session.csrfToken)
      setConfirming(false)
      await connection.refetch()
    } catch (failure) {
      setError(
        failure instanceof ApiFailure && failure.status === 409
          ? "Stop active or uncertain conversations before disconnecting."
          : "Claude could not be disconnected. Check the connection and try again."
      )
    } finally {
      setPending(false)
    }
  }
  return (
    <section className="provider-settings" aria-labelledby={headingId}>
      <h3 id={headingId}>Claude Code</h3>
      <p
        ref={statusElement}
        tabIndex={-1}
        className="provider-settings-status"
        role="status"
        aria-live="polite"
      >
        {pending
          ? translate("Disconnecting Claude…")
          : connection.isFetching && !connection.data
            ? translate("Checking the account…")
            : connected
              ? translate("Connected")
              : connection.data?.status === "connecting"
                ? translate("Waiting for authorization")
                : connection.data?.status === "disconnected"
                  ? translate("Not connected")
                  : (claudeConnectionProblem(connection.data) ??
                    translate("The account status could not be confirmed."))}
      </p>
      {connection.data?.account && (
        <dl className="provider-account">
          <div>
            <dt>{translate("Account")}</dt>
            <dd>
              {connection.data.account.email ??
                translate("Personal Claude account")}
            </dd>
          </div>
          <div>
            <dt>{translate("Plan")}</dt>
            <dd>{connection.data.account.plan}</dd>
          </div>
        </dl>
      )}
      {error && <p role="alert">{translate(error)}</p>}
      {connected ? (
        confirming ? (
          <div className="provider-disconnect-confirmation">
            <p>
              {translate(
                "Disconnect Claude Code? Your files remain. Terminal processes may continue until the environment stops."
              )}{" "}
            </p>
            <div className="provider-actions">
              <Button
                ref={cancelButton}
                variant="secondary"
                disabled={pending}
                onClick={() => setConfirming(false)}
              >
                {translate("Cancel")}{" "}
              </Button>
              <Button
                variant="danger"
                disabled={pending}
                onClick={() => void disconnect()}
              >
                {translate("Disconnect Claude")}{" "}
              </Button>
            </div>
          </div>
        ) : (
          <Button
            ref={disconnectButton}
            variant="ghost"
            disabled={!available || pending}
            onClick={() => setConfirming(true)}
          >
            {translate("Disconnect Claude")}{" "}
          </Button>
        )
      ) : connection.data?.status === "connecting" ? (
        <Button variant="secondary" disabled={!available} onClick={onConnect}>
          {translate("Continue sign-in")}{" "}
        </Button>
      ) : connection.data?.status === "disconnected" ? (
        <Button variant="secondary" disabled={!available} onClick={onConnect}>
          {translate("Connect Claude")}{" "}
        </Button>
      ) : (
        <div className="provider-actions">
          <Button variant="secondary" disabled={!available} onClick={onConnect}>
            {translate("Connect Claude")}{" "}
          </Button>
          <Button
            variant="ghost"
            disabled={!available || connection.isFetching}
            onClick={() => void connection.refetch()}
          >
            {translate("Check again")}{" "}
          </Button>
        </div>
      )}
    </section>
  )
}
