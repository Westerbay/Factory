import { lazy, Suspense, useState } from "react"

const TerminalSession = lazy(() => import("./terminal-session"))

export function TerminalPanel({ available }: { available: boolean }) {
  const [opened, setOpened] = useState(false)
  return (
    <section aria-labelledby="terminal-heading">
      <h2 id="terminal-heading">Terminal</h2>
      <p>
        Your shell opens in ~/Documents. Start your environment above before
        connecting.
      </p>
      {opened && available ? (
        <Suspense fallback={<p role="status">Loading the terminal…</p>}>
          <TerminalSession />
        </Suspense>
      ) : (
        <button
          type="button"
          disabled={!available}
          onClick={() => setOpened(true)}
        >
          Open terminal
        </button>
      )}
      {!available && (
        <p role="status">
          The terminal needs a running environment. Check its status above.
        </p>
      )}
    </section>
  )
}
