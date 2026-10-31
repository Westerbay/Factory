import { useState } from "react"
import { createRoute } from "@tanstack/react-router"

import { CodexCard } from "../features/agent-connections/codex-card"
import { useConnection } from "../features/agent-connections/use-connection"
import { Chat } from "../features/conversations/chat"
import { Recovery } from "../features/conversations/recovery"
import { EnvironmentCard } from "../features/environments/environment-card"
import { useEnvironment } from "../features/environments/use-environment"
import { TerminalPanel } from "../features/terminal/terminal-panel"
import { authenticatedRoute } from "./_authenticated"

export const appRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/app",
  component: Workspace,
})

function Workspace() {
  const environment = useEnvironment()
  const env = environment.state.environment
  const available = env?.status === "running"
  const connection = useConnection(available)
  const connected = connection.state.connection?.status === "connected"
  // Keep the chat mounted while the environment stops so an uncertain run can be recovered.
  const [wasConnected, setWasConnected] = useState(false)
  if (connected && !wasConnected) setWasConnected(true)
  if (
    available &&
    connection.state.connection?.status === "disconnected" &&
    wasConnected
  )
    setWasConnected(false)
  const [terminalVisible, setTerminalVisible] = useState(true)
  const [mobilePane, setMobilePane] = useState<"chat" | "terminal">("chat")
  const [terminalWidth, setTerminalWidth] = useState(40)
  const showChat = connected || (!available && wasConnected)
  const preparing =
    (!env && !environment.state.message) ||
    env?.status === "starting" ||
    environment.state.pending === "initialize"
  return (
    <div className="workspace">
      {!available && preparing ? (
        <section
          className="workspace-preparing"
          role="status"
          aria-live="polite"
        >
          <span className="workspace-spinner" aria-hidden="true" />
          <h1>Preparing your environment</h1>
          <p>Setting up your private workspace. This can take a few moments.</p>
        </section>
      ) : null}
      {!showChat && !preparing && (
        <div className="workspace-onboarding">
          <EnvironmentCard {...environment} />
          {env?.status === "stopped" && <Recovery />}
          {available && <CodexCard available={available} {...connection} />}
        </div>
      )}
      {showChat && (
        <>
          <div className="workspace-controls">
            <details>
              <summary>Environment and account</summary>
              <div className="workspace-settings">
                <EnvironmentCard {...environment} />
                {env?.status === "stopped" && <Recovery />}
                <CodexCard available={available} {...connection} />
              </div>
            </details>
            <button
              type="button"
              className="terminal-toggle"
              onClick={() => setTerminalVisible((value) => !value)}
              aria-expanded={terminalVisible}
              aria-controls="workspace-terminal"
            >
              {terminalVisible ? "Hide terminal" : "Show terminal"}
            </button>
          </div>
          <div
            className="workspace-mobile-tabs"
            role="group"
            aria-label="Workspace view"
          >
            <button
              type="button"
              aria-pressed={mobilePane === "chat"}
              onClick={() => setMobilePane("chat")}
            >
              Chat
            </button>
            <button
              type="button"
              aria-pressed={mobilePane === "terminal"}
              onClick={() => {
                setTerminalVisible(true)
                setMobilePane("terminal")
              }}
            >
              Terminal
            </button>
          </div>
          <div
            className="workspace-split"
            style={{
              gridTemplateColumns: terminalVisible
                ? `minmax(0, 1fr) minmax(18rem, ${terminalWidth}%)`
                : "minmax(0, 1fr)",
            }}
          >
            <div
              className={`workspace-chat-pane ${mobilePane === "terminal" ? "mobile-hidden" : ""}`}
            >
              <Chat available={available} />
            </div>
            <div
              id="workspace-terminal"
              className={`workspace-terminal-pane ${mobilePane === "chat" ? "mobile-hidden" : ""}`}
              hidden={!terminalVisible}
            >
              <div className="terminal-width-control">
                <label>
                  Terminal width{" "}
                  <input
                    aria-label="Terminal width"
                    type="range"
                    min="25"
                    max="65"
                    value={terminalWidth}
                    onChange={(event) =>
                      setTerminalWidth(Number(event.target.value))
                    }
                  />
                </label>
              </div>
              <TerminalPanel available={available} />
            </div>
          </div>
        </>
      )}
    </div>
  )
}
