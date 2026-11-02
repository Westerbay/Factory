import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useEffect, useLayoutEffect, useRef, useState } from "react"

import { Button } from "../../components/ui/button"
import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import {
  listRecoverable,
  recover,
  recoverModelProbe,
  type Recoverable,
} from "./api"
import {
  listRecoverable as listRecoverableImports,
  recover as recoverImport,
  remainingAfterRecovery,
  type RecoverableImport,
} from "../projects/api"
import "./recovery.css"

/** A stopped environment can expose runs to recover even after a page reload. */
export function Recovery() {
  useLocale()
  const { state: session } = useSession()
  const [entries, setEntries] = useState<Recoverable[]>([])
  const [imports, setImports] = useState<RecoverableImport[]>([])
  const [message, setMessage] = useState("")
  const [busy, setBusy] = useState(false)
  const [checking, setChecking] = useState(true)
  const [retryNeeded, setRetryNeeded] = useState(false)
  const [version, setVersion] = useState(0)
  const mutation = useRef<AbortController | null>(null)
  const statusRef = useRef<HTMLParagraphElement>(null)
  const restoreImportFocus = useRef(false)
  useLayoutEffect(() => {
    if (restoreImportFocus.current && document.activeElement === document.body)
      statusRef.current?.focus()
    restoreImportFocus.current = false
  }, [imports])
  useEffect(() => () => mutation.current?.abort(), [])
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  useEffect(() => {
    if (!csrf) return
    const controller = new AbortController()
    setChecking(true)
    const signal = AbortSignal.any([
      controller.signal,
      AbortSignal.timeout(10_000),
    ])
    void Promise.all([listRecoverable(signal), listRecoverableImports(signal)])
      .then(([data, pendingImports]) => {
        if (controller.signal.aborted) return
        setEntries(data)
        setImports(pendingImports)
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
  async function runImport(
    entry: RecoverableImport,
    button: HTMLButtonElement
  ) {
    if (mutation.current || checking || !csrf) return
    setBusy(true)
    setMessage("")
    const controller = new AbortController()
    mutation.current = controller
    try {
      await recoverImport(
        entry.projectId,
        csrf,
        AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
      )
      if (controller.signal.aborted) return
      restoreImportFocus.current = document.activeElement === button
      setMessage("Checking remaining imports…")
      setImports((current) =>
        current.filter((item) => item.projectId !== entry.projectId)
      )
      const remaining = await remainingAfterRecovery(
        imports.filter((item) => item.projectId !== entry.projectId),
        AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
      )
      if (controller.signal.aborted) return
      setImports(remaining)
      setMessage(
        remaining.length
          ? "More imports need recovery before returning to your workspace."
          : "Import recovered. Return to your workspace to retry it."
      )
      setRetryNeeded(false)
    } catch {
      if (!controller.signal.aborted) {
        setMessage(
          "Import recovery could not be confirmed. Check its status and try again."
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
      aria-label={m.workspace_recovery()}
      aria-busy={busy || checking}
    >
      {entries.length > 0 && (
        <div className="workspace-recovery-tasks">
          <h2>
            {entries.length === 1
              ? m.a_task_needs_recovery()
              : m.tasks_need_recovery()}
          </h2>
          <p>{m.recover_previous_tasks_before_returning_to_your_workspace()}</p>
          <ul>
            {entries.map((entry, index) => (
              <li key={entry.runId}>
                {!entry.launchSettled && (
                  <p>
                    {m.task_number_could_not_be_confirmed_as_launched_safe_recovery_requ(
                      { number: index + 1 }
                    )}
                  </p>
                )}
                <Button
                  disabled={busy || checking || !csrf || !entry.launchSettled}
                  onClick={() => {
                    void run(entry)
                  }}
                >
                  {m.recover_task()} {index + 1}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}
      {imports.length > 0 && (
        <div className="workspace-recovery-tasks">
          <h2>
            {imports.length === 1
              ? m.an_import_needs_recovery()
              : m.imports_need_recovery()}
          </h2>
          <p>
            {m.recover_interrupted_imports_before_returning_to_your_workspace()}
          </p>
          <ul>
            {imports.map((entry, index) => (
              <li key={entry.projectId}>
                <Button
                  disabled={checking || !csrf}
                  aria-disabled={busy || undefined}
                  onClick={(event) =>
                    void runImport(entry, event.currentTarget)
                  }
                >
                  {m.recover_import()} {index + 1}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}
      <p
        ref={statusRef}
        tabIndex={-1}
        className="workspace-recovery-status"
        role="status"
      >
        {translateMessage(message)}
      </p>
      <details className="workspace-recovery-options">
        <summary>{m.recovery_options()}</summary>
        <div className="workspace-recovery-options-body">
          <p>
            {m.restore_model_access_if_it_remains_unavailable_after_a_retry()}
          </p>
          <Button
            disabled={busy || checking || !csrf}
            onClick={() => {
              void restoreModels()
            }}
          >
            {m.restore_model_access()}{" "}
          </Button>
          {(retryNeeded || entries.some((entry) => !entry.launchSettled)) && (
            <Button
              variant="ghost"
              disabled={busy || checking}
              onClick={() => setVersion((n) => n + 1)}
            >
              {checking ? m.checking() : m.check_recovery_status()}
            </Button>
          )}
        </div>
      </details>
    </section>
  )
}
