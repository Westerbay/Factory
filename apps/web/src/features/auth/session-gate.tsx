import { useEffect, useState, type ReactNode } from "react"
import { Link, useRouterState } from "@tanstack/react-router"

import { useSession } from "../../lib/auth/session-context"

export function SessionGate({ children }: { children: ReactNode }) {
  const { state, controller } = useSession()
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const [verifiedPath, setVerifiedPath] = useState<string>()
  useEffect(() => {
    let mounted = true
    void controller.refresh().then(() => {
      if (mounted) setVerifiedPath(pathname)
    })
    return () => {
      mounted = false
    }
  }, [controller, pathname])

  if (state.status === "active" && verifiedPath === pathname) return children
  if (
    state.status === "checking" ||
    (state.status === "active" && verifiedPath !== pathname)
  )
    return <p role="status">Checking your access…</p>
  if (state.status === "anonymous")
    return (
      <section>
        <h1>Sign in to continue</h1>
        <p>Your session is missing, expired, or no longer has access.</p>
        <Link to="/sign-in">Continue to sign-in</Link>
      </section>
    )
  if (state.status === "logout-failed")
    return (
      <section>
        <h1>Sign-out could not be confirmed</h1>
        <p>
          Your workspace is hidden. Retry to make sure your server session is
          revoked.
        </p>
        <button type="button" onClick={() => void controller.logout()}>
          Try signing out again
        </button>
      </section>
    )
  return (
    <section>
      <h1>
        {state.status === "offline"
          ? "You are offline"
          : "Access could not be verified"}
      </h1>
      <p>
        Your workspace is hidden until your connection and access can be
        checked.
      </p>
      <button type="button" onClick={() => void controller.refresh()}>
        Check again
      </button>
    </section>
  )
}
