import { useTranslation } from "react-i18next"
import { useEffect, useState, type ReactNode } from "react"
import { Link, useRouterState } from "@tanstack/react-router"

import { Brand } from "../../components/ui/brand"
import { Button } from "../../components/ui/button"
import { LoadingState } from "../../components/ui/loading-state"
import { useSession } from "../../lib/auth/session-context"

export function SessionGate({ children }: { children: ReactNode }) {
  const { t: translate } = useTranslation()
  const { state, controller } = useSession()
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const [verifiedPath, setVerifiedPath] = useState<string>()
  useEffect(() => {
    let mounted = true
    const acceptVerification = () => {
      const current = controller.getSnapshot()
      if (mounted && current.status === "active" && !current.verification)
        setVerifiedPath(pathname)
    }
    // A failed route check may later recover through the global online/focus
    // listener. An unchanged successful check resolves without publishing.
    const unsubscribe = controller.subscribe(acceptVerification)
    void controller.refresh().then(acceptVerification)
    return () => {
      mounted = false
      unsubscribe()
    }
  }, [controller, pathname])

  if (state.status === "active" && verifiedPath === pathname)
    return (
      <div key={controller.getScope()}>
        {children}
        {state.verification && (
          <div className="session-notice" role="status">
            <span>
              {state.verification === "offline"
                ? translate("Offline. Reconnect to continue.")
                : translate("Session check failed.")}
            </span>
            {state.verification === "unavailable" && (
              <Button variant="ghost" onClick={() => void controller.refresh()}>
                {translate("Retry")}{" "}
              </Button>
            )}
          </div>
        )}
      </div>
    )
  let content: ReactNode
  if (
    state.status === "checking" ||
    (state.status === "active" && !state.verification)
  )
    content = <LoadingState title={translate("Checking your access…")} />
  else if (state.status === "anonymous")
    content = (
      <>
        <h1>{translate("Sign in to continue")}</h1>
        <p>
          {translate(
            "Your session is missing, expired, or no longer has access."
          )}
        </p>
        <Link className="ui-button ui-button-primary" to="/sign-in">
          {translate("Continue to sign-in")}{" "}
        </Link>
      </>
    )
  else if (state.status === "logout-failed")
    content = (
      <>
        <h1>{translate("Sign-out could not be confirmed")}</h1>
        <p>
          {translate(
            "Your workspace is hidden. Retry to make sure your server session is revoked."
          )}{" "}
        </p>
        <Button onClick={() => void controller.logout()}>
          {translate("Try signing out again")}{" "}
        </Button>
      </>
    )
  else
    content = (
      <>
        <h1>
          {state.status === "offline" ||
          (state.status === "active" && state.verification === "offline")
            ? translate("You are offline")
            : translate("Access could not be verified")}
        </h1>
        <p>
          {translate(
            "Your workspace is hidden until your connection and access can be checked."
          )}{" "}
        </p>
        <Button onClick={() => void controller.refresh()}>
          {translate("Check again")}
        </Button>
      </>
    )

  return (
    <section className="auth-page">
      <Brand />
      <div className="auth-content">{content}</div>
    </section>
  )
}
