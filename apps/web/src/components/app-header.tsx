import { Link } from "@tanstack/react-router"

import { Brand } from "./ui/brand"
import { Button } from "./ui/button"
import { useSession } from "../lib/auth/session-context"

export function AppHeader() {
  const { state, controller } = useSession()
  if (state.status !== "active") return null

  return (
    <header className="admin-header">
      <Brand />
      <nav className="admin-account-controls" aria-label="Account">
        <span className="admin-account-name">
          {state.session.user.displayName || state.session.user.handle}
        </span>
        <Link className="ui-button ui-button-ghost" to="/app">
          Workspace
        </Link>
        <Button variant="ghost" onClick={() => void controller.logout()}>
          Sign out
        </Button>
      </nav>
    </header>
  )
}
