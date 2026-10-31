import { useState } from "react"

import { Button } from "../components/ui/button"
import { useAppUpdate } from "./use-app-update"
import "./updates.css"

export function AppUpdates() {
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
          Update available
        </Button>
      )
    }
    return (
      <aside
        className="update-prompt"
        aria-labelledby="update-title"
        aria-live="polite"
      >
        <h2 id="update-title">An update is ready</h2>
        <p>
          Reload when you are ready. Unsaved changes on this page may be lost.
        </p>
        <div className="update-actions">
          <Button variant="primary" onClick={applyUpdate}>
            Reload to update
          </Button>
          <Button onClick={() => setDismissed(true)}>Later</Button>
        </div>
      </aside>
    )
  }

  if (error) {
    return (
      <p className="update-error" role="status">
        App updates could not be checked. Reload this page when you are online
        to try again.
      </p>
    )
  }
  return null
}
