import { createRoute, Outlet } from "@tanstack/react-router"

import { SessionGate } from "../features/auth/session-gate"
import { rootRoute } from "./__root"

export const authenticatedRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "authenticated",
  component: () => (
    <SessionGate>
      <Outlet />
    </SessionGate>
  ),
})
