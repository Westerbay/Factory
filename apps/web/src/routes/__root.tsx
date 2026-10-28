import { createRootRoute, Link, Outlet } from "@tanstack/react-router"
import { AppHeader } from "../components/app-header"
import { SessionProvider } from "../lib/auth/session-context"
import "../features/auth/auth.css"

export const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: () => (
    <section>
      <h1>Page not found</h1>
      <p>This page is not available.</p>
      <Link to="/">Return to Factory</Link>
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
        <AppHeader />
        <main id="main" tabIndex={-1}>
          <Outlet />
        </main>
        <footer>Factory · Your private workspace</footer>
      </div>
    </SessionProvider>
  )
}
