import * as m from "@workspace/i18n/messages"
import { useLocale } from "../lib/i18n"
import { Link } from "@tanstack/react-router"

import { Brand } from "./ui/brand"
import { Button } from "./ui/button"
import { useSession } from "../lib/auth/session-context"

export function AppHeader() {
  useLocale()
  const { state, controller } = useSession()
  if (state.status !== "active") return null

  return (
    <header className="admin-header">
      <Brand />
      <nav className="admin-account-controls" aria-label={m.account()}>
        <span className="admin-account-name">
          {state.session.user.displayName || state.session.user.handle}
        </span>
        <Link className="ui-button ui-button-ghost" to="/app">
          {m.workspace()}{" "}
        </Link>
        <Button variant="ghost" onClick={() => void controller.logout()}>
          {m.sign_out()}{" "}
        </Button>
      </nav>
    </header>
  )
}
