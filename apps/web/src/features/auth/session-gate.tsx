import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import { useEffect, useState, type ReactNode } from "react"
import { Link, useRouterState } from "@tanstack/react-router"

import { AppInstallOffer } from "../../pwa/app-install"
import { Brand } from "../../components/ui/brand"
import { Button } from "../../components/ui/button"
import { LoadingState } from "../../components/ui/loading-state"
import { useSession } from "../../lib/auth/session-context"

export function SessionGate({
  children,
  verifyOnEntry = false,
}: {
  children: ReactNode
  verifyOnEntry?: boolean
}) {
  useLocale()
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
    const unsubscribe = verifyOnEntry
      ? controller.subscribe(acceptVerification)
      : undefined
    void controller.refresh().then(() => {
      if (verifyOnEntry) acceptVerification()
    })
    return () => {
      mounted = false
      unsubscribe?.()
    }
  }, [controller, pathname, verifyOnEntry])

  const expired =
    state.status === "active" &&
    Date.parse(state.session.expiresAt) <= Date.now()
  if (
    state.status === "active" &&
    !expired &&
    (!verifyOnEntry || verifiedPath === pathname)
  )
    return (
      <div key={controller.getScope()}>
        {children}
        {state.verification && !verifyOnEntry && (
          <div className="session-notice" role="status">
            <span>
              {state.verification === "offline"
                ? m.offline_reconnect_to_continue()
                : m.session_check_failed()}
            </span>
            {state.verification === "unavailable" && (
              <Button variant="ghost" onClick={() => void controller.refresh()}>
                {m.retry()}{" "}
              </Button>
            )}
          </div>
        )}
      </div>
    )
  let content: ReactNode
  if (
    state.status === "checking" ||
    expired ||
    (state.status === "active" && !state.verification)
  )
    content = <LoadingState title={m.checking_your_access()} />
  else if (state.status === "anonymous")
    content = (
      <>
        <h1>{m.sign_in_to_continue()}</h1>
        <p>{m.your_session_is_missing_expired_or_no_longer_has_access()}</p>
        <Link className="ui-button ui-button-primary" to="/sign-in">
          {m.continue_to_sign_in()}{" "}
        </Link>
        <AppInstallOffer />
      </>
    )
  else if (state.status === "logout-failed")
    content = (
      <>
        <h1>{m.sign_out_could_not_be_confirmed()}</h1>
        <p>
          {m.your_workspace_is_hidden_retry_to_make_sure_your_server_session_i()}{" "}
        </p>
        <Button onClick={() => void controller.logout()}>
          {m.try_signing_out_again()}{" "}
        </Button>
      </>
    )
  else
    content = (
      <>
        <h1>
          {state.status === "offline" ||
          (state.status === "active" && state.verification === "offline")
            ? m.you_are_offline()
            : m.access_could_not_be_verified()}
        </h1>
        <p>
          {m.your_workspace_is_hidden_until_your_connection_and_access_can_be()}{" "}
        </p>
        <Button onClick={() => void controller.refresh()}>
          {m.check_again()}
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
