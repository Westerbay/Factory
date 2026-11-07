import { createRoute } from "@tanstack/react-router"

import { WorkspacePage } from "../components/workspace/workspace-page"
import { authenticatedRoute } from "./_authenticated"

export type WorkspaceSearch = {
  view?: "overview" | "chat" | "features"
  feature?: string
}

function workspaceSearch(search: Record<string, unknown>): WorkspaceSearch {
  const view =
    search.view === "chat" || search.view === "features"
      ? search.view
      : "overview"
  const feature =
    typeof search.feature === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      search.feature
    )
      ? search.feature
      : undefined
  return { view, ...(view === "features" && feature ? { feature } : {}) }
}

export const appRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/app",
  component: WorkspacePage,
  validateSearch: workspaceSearch,
})
