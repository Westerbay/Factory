import { createRootRoute, Link, Outlet } from "@tanstack/react-router"

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
    <div className="shell">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="header">
        <Link className="brand" to="/">
          Factory
        </Link>
        <span className="preview-label">Local preview</span>
      </header>
      <main id="main" tabIndex={-1}>
        <Outlet />
      </main>
      <footer>Factory · Your private workspace</footer>
    </div>
  )
}
