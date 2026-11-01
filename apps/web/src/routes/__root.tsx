import { useTranslation } from "react-i18next"
import { createRootRoute, Link, Outlet } from "@tanstack/react-router"
import { Brand } from "../components/ui/brand"
import { SessionProvider } from "../lib/auth/session-context"
import "../features/auth/auth.css"

export const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFound,
})

function NotFound() {
  const { t: translate } = useTranslation()
  return (
    <section className="auth-page">
      <Brand />
      <div className="auth-content">
        <h1>{translate("Page not found")}</h1>
        <p>{translate("This page is not available.")}</p>
        <Link className="ui-button ui-button-secondary" to="/app">
          {translate("Return to Factory")}
        </Link>
      </div>
    </section>
  )
}

function RootLayout() {
  const { t: translate } = useTranslation()
  return (
    <SessionProvider>
      <div className="shell">
        <a className="skip-link" href="#main">
          {translate("Skip to content")}{" "}
        </a>
        <main id="main" tabIndex={-1}>
          <Outlet />
        </main>
      </div>
    </SessionProvider>
  )
}
