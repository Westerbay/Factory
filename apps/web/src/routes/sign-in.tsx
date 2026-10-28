import { createRoute } from "@tanstack/react-router"

import { SignInView, signInReason } from "../features/auth/sign-in-view"
import { rootRoute } from "./__root"

export const signInRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sign-in",
  validateSearch: (search): { reason?: ReturnType<typeof signInReason> } => ({
    reason: signInReason(search.reason),
  }),
  component: SignInPage,
})

function SignInPage() {
  return <SignInView reason={signInRoute.useSearch().reason} />
}
