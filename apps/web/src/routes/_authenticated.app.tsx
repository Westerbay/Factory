import { createRoute } from "@tanstack/react-router"

import { CodexCard } from "../features/agent-connections/codex-card"
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
  const available = environment.state.environment?.status === "running"
  return (
    <>
      <EnvironmentCard {...environment} />
      <TerminalPanel
        key={available ? "running" : "unavailable"}
        available={available}
      />
      <CodexCard available={available} />
    </>
  )
}
