import { createRootRoute, Link, Outlet } from "@tanstack/react-router"
import { Brand } from "../components/ui/brand"
import { SessionProvider } from "../lib/auth/session-context"
import "../features/auth/auth.css"

export const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: () => (
    <section className="auth-page">
      <Brand />
      <div className="auth-content">
        <h1>Page not found</h1>
        <p>This page is not available.</p>
        <Link className="ui-button ui-button-secondary" to="/app">
          Return to Factory
        </Link>
      </div>
    </section>
  ),
})

function RootLayout() {
  return (
    <SessionProvider>
      <div className="shell">
        <a className="skip-link" href="#main">
          Skip to content
        </a>
        <main id="main" tabIndex={-1}>
          <Outlet />
        </main>
      </div>
    </SessionProvider>
  )
}
