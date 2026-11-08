import * as m from "@workspace/i18n/messages"
import { useLocale } from "../lib/i18n"
import { useEffect, useRef, useState } from "react"

import { Button } from "../components/ui/button"
import { useAppUpdate } from "./use-app-update"
import "./updates.css"

interface InstallPromptEvent extends Event {
  prompt(): Promise<unknown>
}

export function AppUpdates() {
  useLocale()
  const { available, error, applyUpdate } = useAppUpdate()
  const [dismissed, setDismissed] = useState(false)
  const [installPrompt, setInstallPrompt] = useState<InstallPromptEvent | null>(
    null
  )
  const installDismissed = useRef(false)

  useEffect(() => {
    if (window.matchMedia("(display-mode: standalone)").matches) return
    function offerInstallation(event: Event) {
      event.preventDefault()
      if (!installDismissed.current)
        setInstallPrompt(event as InstallPromptEvent)
    }
    function clearInstallation() {
      installDismissed.current = true
      setInstallPrompt(null)
    }
    window.addEventListener("beforeinstallprompt", offerInstallation)
    window.addEventListener("appinstalled", clearInstallation)
    return () => {
      window.removeEventListener("beforeinstallprompt", offerInstallation)
      window.removeEventListener("appinstalled", clearInstallation)
    }
  }, [])

  function dismissInstallation() {
    installDismissed.current = true
    setInstallPrompt(null)
  }

  if (available) {
    if (dismissed) {
      return (
        <Button
          className="update-reminder"
          type="button"
          onClick={() => setDismissed(false)}
        >
          {m.update_available()}{" "}
        </Button>
      )
    }
    return (
      <aside
        className="update-prompt"
        aria-labelledby="update-title"
        aria-live="polite"
      >
        <h2 id="update-title">{m.an_update_is_ready()}</h2>
        <p>
          {m.reload_when_you_are_ready_unsaved_changes_on_this_page_may_be_los()}{" "}
        </p>
        <div className="update-actions">
          <Button variant="primary" onClick={applyUpdate}>
            {m.reload_to_update()}{" "}
          </Button>
          <Button onClick={() => setDismissed(true)}>{m.later()}</Button>
        </div>
      </aside>
    )
  }

  if (installPrompt) {
    return (
      <aside className="update-prompt" aria-labelledby="install-title">
        <h2 id="install-title">{m.install_factory()}</h2>
        <div className="update-actions">
          <Button
            variant="primary"
            onClick={() => {
              if (installDismissed.current) return
              dismissInstallation()
              void installPrompt.prompt().catch(() => undefined)
            }}
          >
            {m.install_app()}
          </Button>
          <Button onClick={dismissInstallation}>{m.later()}</Button>
        </div>
      </aside>
    )
  }

  if (error) {
    return (
      <p className="update-error" role="status">
        {m.app_updates_could_not_be_checked_reload_this_page_when_you_are_on()}{" "}
      </p>
    )
  }
  return null
}
