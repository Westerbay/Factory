import * as m from "@workspace/i18n/messages"
import { useLocale } from "../lib/i18n"
import { createRootRoute, Link, Outlet } from "@tanstack/react-router"
import { Brand } from "../components/ui/brand"
import { SessionProvider } from "../lib/auth/session-context"
import "../features/auth/auth.css"

export const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFound,
})

function NotFound() {
  useLocale()
  return (
    <section className="auth-page">
      <Brand />
      <div className="auth-content">
        <h1>{m.page_not_found()}</h1>
        <p>{m.this_page_is_not_available()}</p>
        <Link className="ui-button ui-button-secondary" to="/app">
          {m.return_to_factory()}
        </Link>
      </div>
    </section>
  )
}

function RootLayout() {
  useLocale()
  return (
    <SessionProvider>
      <div className="shell">
        <a className="skip-link" href="#main">
          {m.skip_to_content()}{" "}
        </a>
        <main id="main" tabIndex={-1}>
          <Outlet />
        </main>
      </div>
    </SessionProvider>
  )
}
