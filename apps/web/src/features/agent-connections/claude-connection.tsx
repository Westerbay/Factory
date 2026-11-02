import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
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
  useLocale()
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
          ? m.disconnecting_claude()
          : connection.isFetching && !connection.data
            ? m.checking_the_account()
            : connected
              ? m.connected()
              : connection.data?.status === "connecting"
                ? m.waiting_for_authorization_738e149b()
                : connection.data?.status === "disconnected"
                  ? m.not_connected()
                  : (claudeConnectionProblem(connection.data) ??
                    m.the_account_status_could_not_be_confirmed())}
      </p>
      {connection.data?.account && (
        <dl className="provider-account">
          <div>
            <dt>{m.account()}</dt>
            <dd>
              {connection.data.account.email ?? m.personal_claude_account()}
            </dd>
          </div>
          <div>
            <dt>{m.plan()}</dt>
            <dd>{connection.data.account.plan}</dd>
          </div>
        </dl>
      )}
      {error && <p role="alert">{translateMessage(error)}</p>}
      {connected ? (
        confirming ? (
          <div className="provider-disconnect-confirmation">
            <p>
              {m.disconnect_claude_code_your_files_remain_terminal_processes_may_c()}{" "}
            </p>
            <div className="provider-actions">
              <Button
                ref={cancelButton}
                variant="secondary"
                disabled={pending}
                onClick={() => setConfirming(false)}
              >
                {m.cancel()}{" "}
              </Button>
              <Button
                variant="danger"
                disabled={pending}
                onClick={() => void disconnect()}
              >
                {m.disconnect_claude()}{" "}
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
            {m.disconnect_claude()}{" "}
          </Button>
        )
      ) : connection.data?.status === "connecting" ? (
        <Button variant="secondary" disabled={!available} onClick={onConnect}>
          {m.continue_sign_in()}{" "}
        </Button>
      ) : connection.data?.status === "disconnected" ? (
        <Button variant="secondary" disabled={!available} onClick={onConnect}>
          {m.connect_claude()}{" "}
        </Button>
      ) : (
        <div className="provider-actions">
          <Button variant="secondary" disabled={!available} onClick={onConnect}>
            {m.connect_claude()}{" "}
          </Button>
          <Button
            variant="ghost"
            disabled={!available || connection.isFetching}
            onClick={() => void connection.refetch()}
          >
            {m.check_again()}{" "}
          </Button>
        </div>
      )}
    </section>
  )
}
