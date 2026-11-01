import { lazy, Suspense, useEffect, useRef, useState } from "react"
import "./terminal.css"

const TerminalSession = lazy(() => import("./terminal-session"))

export function TerminalPanel({
  available,
  conversationId,
  projectName,
  projectWarnings,
  visible = true,
  expanded,
  onExpandedChange,
}: {
  available: boolean
  conversationId?: string
  projectName?: string
  projectWarnings?: string[]
  visible?: boolean
  expanded: boolean
  onExpandedChange: (expanded: boolean) => void
}) {
  const [opened, setOpened] = useState(visible)
  const detached = useRef(new Map<string, number>())
  const terminalKey = conversationId ?? "documents"

  useEffect(() => {
    if (visible) setOpened(true)
  }, [visible])

  return (
    <section className="terminal-pane" aria-label="Terminal">
      {projectWarnings?.map((warning) => (
        <p key={warning} className="terminal-project-warning" role="status">
          {warning}
        </p>
      ))}
      {opened && available ? (
        <Suspense
          fallback={
            <p className="terminal-status" role="status">
              Loading terminal…
            </p>
          }
        >
          <TerminalSession
            key={terminalKey}
            conversationId={conversationId}
            projectName={projectName}
            onOwnDetach={() => {
              // Bound memory to terminals actually switched in this panel.
              detached.current.set(terminalKey, Date.now())
              if (detached.current.size > 100) {
                const oldest = detached.current.keys().next().value
                if (oldest !== undefined) detached.current.delete(oldest)
              }
            }}
            consumeRecentOwnDetach={() => {
              const last = detached.current.get(terminalKey)
              detached.current.delete(terminalKey)
              return last !== undefined && Date.now() - last < 5000
            }}
            expanded={expanded}
            onExpandedChange={onExpandedChange}
          />
        </Suspense>
      ) : !available ? (
        <p className="terminal-status" role="status">
          Waiting for your environment…
        </p>
      ) : null}
    </section>
  )
}
