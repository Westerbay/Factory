import { lazy, Suspense, useEffect, useState } from "react"
import "./terminal.css"

const TerminalSession = lazy(() => import("./terminal-session"))

export function TerminalPanel({
  available,
  visible = true,
  expanded,
  onExpandedChange,
}: {
  available: boolean
  visible?: boolean
  expanded: boolean
  onExpandedChange: (expanded: boolean) => void
}) {
  const [opened, setOpened] = useState(visible)

  useEffect(() => {
    if (visible) setOpened(true)
  }, [visible])

  return (
    <section className="terminal-pane" aria-label="Terminal">
      {opened && available ? (
        <Suspense
          fallback={
            <p className="terminal-status" role="status">
              Loading terminal…
            </p>
          }
        >
          <TerminalSession
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
