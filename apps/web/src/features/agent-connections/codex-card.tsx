import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react"

import { useSession } from "../../lib/auth/session-context"
import {
  changeConnection,
  canDisconnectConnection,
  connectionProblem,
  readConnection,
  type Connection,
  type LoginChallenge,
} from "./api"
import { ConnectionController } from "./connection-controller"
import "./connections.css"

const labels: Record<Connection["status"], string> = {
  unavailable: "Unavailable",
  disconnected: "Not connected",
  connecting: "Waiting for authorization",
  connected: "Connected",
  error: "Needs attention",
}

function DeviceCode({ challenge }: { challenge: LoginChallenge }) {
  const [message, setMessage] = useState("")
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])
  async function copy() {
    try {
      await navigator.clipboard.writeText(challenge.userCode)
      if (mounted.current) setMessage("Code copied.")
    } catch {
      if (mounted.current)
        setMessage("Copy was unavailable. Select and copy the code above.")
    }
  }
  return (
    <div className="codex-challenge">
      <p>
        Enter this code on the official OpenAI page to connect your personal
        account.
      </p>
      <p>
        <span className="codex-code-label">Authorization code</span>
        <code className="codex-code">{challenge.userCode}</code>
      </p>
      <div className="codex-actions">
        <button type="button" onClick={copy}>
          Copy code
        </button>
        <a
          href={challenge.verificationUrl}
          target="_blank"
          rel="noopener noreferrer"
        >
          Continue on OpenAI <span className="codex-new-tab">(new tab)</span>
        </a>
      </div>
      <p>
        Expires at{" "}
        <time dateTime={challenge.expiresAt}>
          {new Date(challenge.expiresAt).toLocaleTimeString()}
        </time>
        . Keep the code private.
      </p>
      <p role="status" aria-live="polite">
        {message}
      </p>
    </div>
  )
}

export function CodexCard({
  available,
  unavailableReason,
}: {
  available: boolean
  unavailableReason?: string
}) {
  const { controller: session } = useSession()
  const statusElement = useRef<HTMLParagraphElement>(null)
  const controller = useMemo(
    () =>
      new ConnectionController({
        session,
        visible: () => document.visibilityState === "visible",
        online: () => navigator.onLine,
        available: () => available,
        read: readConnection,
        change: changeConnection,
      }),
    [session, available]
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
  const connection = state.connection
  const busy = state.pending !== null && state.pending !== "read"
  const challenge = connection?.challenge
  const canLogin =
    connection?.status === "disconnected" || connection?.status === "error"
  const canLogout = canDisconnectConnection(connection)
  const operationLabel =
    connection?.errorCode === "cancelled"
      ? "Sign-in cancelled"
      : connection?.errorCode === "expired"
        ? "Sign-in expired"
        : undefined
  function act(action: () => Promise<void>) {
    // Action buttons can disappear when state changes; retain a meaningful focus target.
    statusElement.current?.focus()
    void action()
  }
  const status =
    state.pending === "login"
      ? "Starting sign-in"
      : state.pending === "cancel"
        ? "Cancelling sign-in"
        : state.pending === "logout"
          ? "Disconnecting"
          : connection
            ? (operationLabel ?? labels[connection.status])
            : state.pending
              ? "Checking"
              : "Unknown"
  return (
    <section className="codex-section" aria-labelledby="codex-heading">
      <h2 id="codex-heading">Codex account</h2>
      <p>
        Connect your personal ChatGPT/Codex subscription inside your private
        environment. Factory never asks for your password or an API key.
      </p>
      {!available ? (
        <p role="status">
          {unavailableReason ??
            "A running environment is required. Check its status above, then start it if needed."}
        </p>
      ) : (
        <div className="connection-panel codex-card">
          <p
            ref={statusElement}
            tabIndex={-1}
            className="codex-status"
            role="status"
            aria-live="polite"
            aria-atomic="true"
          >
            {status}
          </p>
          {connection?.account && (
            <dl className="codex-account">
              <div>
                <dt>Account</dt>
                <dd>
                  {connection.account.email ?? "Personal ChatGPT account"}
                </dd>
              </div>
              <div>
                <dt>Plan reported by Codex</dt>
                <dd>{connection.account.plan}</dd>
              </div>
            </dl>
          )}
          {challenge && (
            <DeviceCode key={challenge.operationId} challenge={challenge} />
          )}
          <p
            className="codex-feedback"
            role="status"
            aria-live="polite"
            aria-atomic="true"
          >
            {state.message ??
              connectionProblem(connection?.errorCode) ??
              (state.paused
                ? "Updates are paused while this page is hidden, offline or your session is being checked."
                : connection?.status === "disconnected"
                  ? "No personal account is connected in this environment."
                  : connection?.status === "connected"
                    ? "Account confirmed. Model access depends on your subscription and permissions."
                    : connection?.status === "error"
                      ? "The account status is uncertain. Refresh before trying again."
                      : "")}
          </p>
          <div className="codex-actions" aria-label="Codex account actions">
            {canLogin && (
              <button
                type="button"
                disabled={busy || state.paused}
                onClick={() => act(controller.login)}
              >
                Connect Codex
              </button>
            )}
            {challenge && (
              <button
                type="button"
                disabled={busy || state.paused}
                onClick={() => act(controller.cancelLogin)}
              >
                Cancel sign-in
              </button>
            )}
            {canLogout && (
              <button
                type="button"
                disabled={busy || state.paused}
                onClick={() => act(controller.logout)}
              >
                Disconnect Codex
              </button>
            )}
            <button
              type="button"
              disabled={busy || state.paused}
              onClick={controller.refresh}
            >
              Refresh account status
            </button>
          </div>
          {canLogout && connection?.status !== "connected" && (
            <p>
              You can still try Disconnect Codex to remove any saved sign-in
              locally, even when its account status cannot be verified.
            </p>
          )}
          {connection?.observedAt && (
            <p className="codex-observed">
              Last checked{" "}
              <time dateTime={connection.observedAt}>
                {new Date(connection.observedAt).toLocaleString()}
              </time>
            </p>
          )}
        </div>
      )}
      <p className="connection-help">
        Disconnecting Codex removes its saved sign-in from this environment and
        keeps your projects. It may not stop a Codex process you already
        launched in the terminal. Use Stop environment to stop all running
        processes. Signing out of Factory does not disconnect your saved Codex
        account.
      </p>
    </section>
  )
}
