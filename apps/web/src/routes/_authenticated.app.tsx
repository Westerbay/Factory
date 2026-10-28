import { createRoute } from "@tanstack/react-router"

import { MemberHome } from "../features/auth/member-home"
import { authenticatedRoute } from "./_authenticated"

export const appRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/app",
  component: MemberHome,
})
