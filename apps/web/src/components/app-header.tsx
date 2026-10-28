import { Link } from "@tanstack/react-router"

import { useSession } from "../lib/auth/session-context"

export function AppHeader() {
  const { state, controller } = useSession()
  return (
    <header className="header">
      <Link className="brand" to="/">
        Factory
      </Link>
      {state.status === "active" && (
        <div className="account-controls">
          <span className="account-name">
            {state.session.user.displayName || state.session.user.handle}
          </span>
          <Link to="/app">Workspace</Link>
          <button type="button" onClick={() => void controller.logout()}>
            Sign out
          </button>
        </div>
      )}
      {state.status === "logout-failed" && (
        <button type="button" onClick={() => void controller.logout()}>
          Retry sign-out
        </button>
      )}
    </header>
  )
}
