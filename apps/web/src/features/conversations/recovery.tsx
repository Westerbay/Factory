import { useEffect, useRef, useState } from "react"

import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import {
  listRecoverable,
  recover,
  recoverModelProbe,
  type Recoverable,
} from "./api"

/** A stopped environment can expose runs to recover even after a page reload. */
export function Recovery() {
  const { state: session } = useSession()
  const [entries, setEntries] = useState<Recoverable[]>([])
  const [message, setMessage] = useState("")
  const [busy, setBusy] = useState(false)
  const [version, setVersion] = useState(0)
  const mutation = useRef<AbortController | null>(null)
  useEffect(() => () => mutation.current?.abort(), [])
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  useEffect(() => {
    const controller = new AbortController()
    void listRecoverable(controller.signal)
      .then((data) => {
        if (controller.signal.aborted) return
        setEntries(data)
        setMessage("")
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          setEntries([])
          setMessage("Recovery status is unavailable. Refresh to check again.")
        }
      })
    return () => controller.abort()
  }, [version])
  async function run(entry: Recoverable) {
    if (busy || !csrf) return
    setBusy(true)
    setMessage("")
    const controller = new AbortController()
    mutation.current = controller
    try {
      await recover(entry.conversationId, entry.runId, csrf, controller.signal)
      if (controller.signal.aborted) return
      setEntries((current) =>
        current.filter((item) => item.runId !== entry.runId)
      )
      setMessage("Run recovered. You can start your environment again.")
    } catch {
      if (!controller.signal.aborted)
        setMessage("Recovery could not be confirmed. Refresh and try again.")
    } finally {
      if (!controller.signal.aborted) setBusy(false)
      if (mutation.current === controller) mutation.current = null
    }
  }
  async function restoreModels() {
    if (busy || !csrf) return
    setBusy(true)
    setMessage("")
    const controller = new AbortController()
    mutation.current = controller
    try {
      await recoverModelProbe(csrf, controller.signal)
      if (!controller.signal.aborted)
        setMessage(
          "Model access recovery checked. You can start your environment again."
        )
    } catch (error) {
      if (!controller.signal.aborted)
        setMessage(
          error instanceof ApiFailure && error.status === 409
            ? "Recovery is blocked: the stopped environment has not been verified, or a model launch is still unconfirmed. Refresh the status or contact your administrator."
            : "Model recovery could not be confirmed. Refresh and try again."
        )
    } finally {
      if (!controller.signal.aborted) setBusy(false)
      if (mutation.current === controller) mutation.current = null
    }
  }
  return (
    <section className="chat-notice" aria-labelledby="recovery-heading">
      <h2 id="recovery-heading">Recovery</h2>
      {entries.length > 0 && (
        <p>
          A previous run must be resolved before another turn can start. Your
          environment remains stopped.
        </p>
      )}
      {entries.map((entry, index) => (
        <div key={entry.runId}>
          {!entry.launchSettled && (
            <p role="status">
              Run {index + 1} is blocked because its Docker launch was not
              confirmed. A safe automatic recovery is not currently available.
              Contact your administrator.
            </p>
          )}
          <button
            type="button"
            disabled={busy || !entry.launchSettled}
            onClick={() => {
              void run(entry)
            }}
          >
            Recover run {index + 1}
          </button>
        </div>
      ))}
      <p>
        If model access remains unavailable after retrying, restore it while the
        environment is stopped.
      </p>
      <button
        type="button"
        disabled={busy}
        onClick={() => {
          void restoreModels()
        }}
      >
        Restore model access
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={() => setVersion((n) => n + 1)}
      >
        Refresh recovery status
      </button>
      <p role="status">{message}</p>
    </section>
  )
}
