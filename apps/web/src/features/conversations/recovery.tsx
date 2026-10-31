import { useEffect, useRef, useState } from "react"

import { Button } from "../../components/ui/button"
import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import {
  listRecoverable,
  recover,
  recoverModelProbe,
  type Recoverable,
} from "./api"
import "./recovery.css"

/** A stopped environment can expose runs to recover even after a page reload. */
export function Recovery() {
  const { state: session } = useSession()
  const [entries, setEntries] = useState<Recoverable[]>([])
  const [message, setMessage] = useState("")
  const [busy, setBusy] = useState(false)
  const [checking, setChecking] = useState(true)
  const [retryNeeded, setRetryNeeded] = useState(false)
  const [version, setVersion] = useState(0)
  const mutation = useRef<AbortController | null>(null)
  useEffect(() => () => mutation.current?.abort(), [])
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  useEffect(() => {
    if (!csrf) return
    const controller = new AbortController()
    setChecking(true)
    void listRecoverable(
      AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
    )
      .then((data) => {
        if (controller.signal.aborted) return
        setEntries(data)
        setMessage("")
        setRetryNeeded(false)
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          setMessage(
            "Recovery status is unavailable. Try again in Recovery options."
          )
          setRetryNeeded(true)
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setChecking(false)
      })
    return () => controller.abort()
  }, [csrf, version])
  async function run(entry: Recoverable) {
    if (mutation.current || checking || !csrf || !entry.launchSettled) return
    setBusy(true)
    setMessage("")
    const controller = new AbortController()
    mutation.current = controller
    try {
      await recover(
        entry.conversationId,
        entry.runId,
        csrf,
        AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
      )
      if (controller.signal.aborted) return
      setEntries((current) =>
        current.filter((item) => item.runId !== entry.runId)
      )
      setMessage("Task recovered.")
      setRetryNeeded(false)
    } catch {
      if (!controller.signal.aborted) {
        setMessage(
          "Recovery could not be confirmed. Check its status in Recovery options."
        )
        setRetryNeeded(true)
      }
    } finally {
      if (!controller.signal.aborted) setBusy(false)
      if (mutation.current === controller) mutation.current = null
    }
  }
  async function restoreModels() {
    if (mutation.current || checking || !csrf) return
    setBusy(true)
    setMessage("")
    const controller = new AbortController()
    mutation.current = controller
    try {
      await recoverModelProbe(
        csrf,
        AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
      )
      if (!controller.signal.aborted) {
        setMessage("Model access checked. You can return to your workspace.")
        setRetryNeeded(false)
      }
    } catch (error) {
      if (!controller.signal.aborted) {
        setMessage(
          error instanceof ApiFailure && error.status === 409
            ? "Recovery is blocked: shutdown or a model launch is still unconfirmed. Check recovery status or contact your administrator."
            : "Model access recovery could not be confirmed. Check its status in Recovery options."
        )
        setRetryNeeded(true)
      }
    } finally {
      if (!controller.signal.aborted) setBusy(false)
      if (mutation.current === controller) mutation.current = null
    }
  }
  return (
    <section
      className="workspace-recovery"
      aria-label="Workspace recovery"
      aria-busy={busy || checking}
    >
      {entries.length > 0 && (
        <div className="workspace-recovery-tasks">
          <h2>
            {entries.length === 1
              ? "A task needs recovery"
              : "Tasks need recovery"}
          </h2>
          <p>Recover previous tasks before returning to your workspace.</p>
          <ul>
            {entries.map((entry, index) => (
              <li key={entry.runId}>
                {!entry.launchSettled && (
                  <p>
                    Task {index + 1} could not be confirmed as launched. Safe
                    recovery requires administrator help.
                  </p>
                )}
                <Button
                  disabled={busy || checking || !csrf || !entry.launchSettled}
                  onClick={() => {
                    void run(entry)
                  }}
                >
                  Recover task {index + 1}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}
      <p className="workspace-recovery-status" role="status">
        {message}
      </p>
      <details className="workspace-recovery-options">
        <summary>Recovery options</summary>
        <div className="workspace-recovery-options-body">
          <p>Restore model access if it remains unavailable after a retry.</p>
          <Button
            disabled={busy || checking || !csrf}
            onClick={() => {
              void restoreModels()
            }}
          >
            Restore model access
          </Button>
          {(retryNeeded || entries.some((entry) => !entry.launchSettled)) && (
            <Button
              variant="ghost"
              disabled={busy || checking}
              onClick={() => setVersion((n) => n + 1)}
            >
              {checking ? "Checking…" : "Check recovery status"}
            </Button>
          )}
        </div>
      </details>
    </section>
  )
}
