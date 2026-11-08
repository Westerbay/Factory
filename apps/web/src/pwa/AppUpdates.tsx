import * as m from "@workspace/i18n/messages"
import { useLocale } from "../lib/i18n"
import { useState } from "react"

import { Button } from "../components/ui/button"
import { useAppUpdate } from "./use-app-update"
import "./updates.css"

export function AppUpdates() {
  useLocale()
  const { available, error, applyUpdate } = useAppUpdate()
  const [dismissed, setDismissed] = useState(false)

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

  if (error) {
    return (
      <p className="update-error" role="status">
        {m.app_updates_could_not_be_checked_reload_this_page_when_you_are_on()}{" "}
      </p>
    )
  }
  return null
}
