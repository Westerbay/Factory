import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import { useEffect, useId, useRef } from "react"
import { Link } from "@tanstack/react-router"
import {
  ChevronUp,
  Download,
  LogOut,
  Power,
  Settings,
  Users,
} from "lucide-react"

import { setTheme, useTheme } from "../../lib/theme/theme"
import { useSession } from "../../lib/auth/session-context"
import { useAppInstall } from "../../pwa/app-install"
import { Button } from "../ui/button"

export function ProfileMenu({
  canStop,
  onSettings,
  onStop,
}: {
  canStop: boolean
  onSettings: () => void
  onStop: () => void
}) {
  useLocale()
  const { state, controller } = useSession()
  const install = useAppInstall()?.install
  const theme = useTheme()
  const themeGroup = useId()
  const ref = useRef<HTMLDetailsElement>(null)
  useEffect(() => {
    function dismiss(event: PointerEvent) {
      if (
        ref.current &&
        !document.querySelector("dialog[open]") &&
        !ref.current.contains(event.target as Node)
      )
        ref.current.open = false
    }
    function escape(event: KeyboardEvent) {
      if (
        event.key === "Escape" &&
        ref.current?.open &&
        !document.querySelector("dialog[open]")
      ) {
        ref.current.open = false
        ref.current.querySelector("summary")?.focus()
      }
    }
    document.addEventListener("pointerdown", dismiss)
    document.addEventListener("keydown", escape)
    return () => {
      document.removeEventListener("pointerdown", dismiss)
      document.removeEventListener("keydown", escape)
    }
  }, [])
  if (state.status !== "active") return null
  const user = state.session.user
  return (
    <details className="profile-menu" ref={ref}>
      <summary
        aria-label={m.profile_options_for_handle({
          handle: user.handle,
        })}
      >
        <span className="profile-avatar">
          <img src="/brand/github.svg" alt="" width="20" height="20" />
        </span>
        <span className="profile-handle">{user.handle}</span>
        <ChevronUp size={14} aria-hidden="true" />
      </summary>
      <div className="profile-options">
        <Button variant="ghost" onClick={onSettings}>
          <Settings aria-hidden="true" />
          {m.settings()}
        </Button>
        {user.role === "admin" && (
          <Link to="/admin/users">
            <Users size={16} aria-hidden="true" />
            {m.user_access()}
          </Link>
        )}
        {install && (
          <Button variant="ghost" onClick={install}>
            <Download aria-hidden="true" />
            {m.install_app()}
          </Button>
        )}
        <Button variant="ghost" disabled={!canStop} onClick={onStop}>
          <Power aria-hidden="true" />
          {m.stop_environment()}
        </Button>
        <div className="profile-menu-divider" />
        <fieldset className="profile-theme">
          <legend>{m.theme()}</legend>
          {(["light", "dark"] as const).map((choice) => (
            <label key={choice}>
              <input
                type="radio"
                name={themeGroup}
                value={choice}
                checked={theme === choice}
                onChange={() => setTheme(choice)}
              />
              {choice === "light" ? m.light() : m.dark()}
            </label>
          ))}
        </fieldset>
        <div className="profile-menu-divider" />
        <Button variant="ghost" onClick={() => void controller.logout()}>
          <LogOut aria-hidden="true" />
          {m.sign_out()}
        </Button>
      </div>
    </details>
  )
}
