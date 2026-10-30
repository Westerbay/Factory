import { createRoute } from "@tanstack/react-router"

import { CodexCard } from "../features/agent-connections/codex-card"
import { environmentUpgradeMessage } from "../features/environments/api"
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
  const upgradeRequired = environment.state.environment?.upgradeRequired
  const available =
    environment.state.environment?.status === "running" && !upgradeRequired
  const unavailableReason = upgradeRequired
    ? environmentUpgradeMessage
    : undefined
  return (
    <>
      <EnvironmentCard {...environment} />
      <TerminalPanel
        key={available ? "running" : "unavailable"}
        available={available}
        unavailableReason={unavailableReason}
      />
      <CodexCard available={available} unavailableReason={unavailableReason} />
    </>
  )
}
