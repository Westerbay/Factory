import { createRoute } from "@tanstack/react-router"

import { AdminUsersPage } from "../features/admin-users/admin-users-page"
import { parseUsersSearch } from "../features/admin-users/api"
import { authenticatedRoute } from "./_authenticated"

export const adminUsersRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: "/admin/users",
  validateSearch: parseUsersSearch,
  component: AdminUsersRoute,
})

function AdminUsersRoute() {
  const { status = "pending", cursor } = adminUsersRoute.useSearch()
  const navigate = adminUsersRoute.useNavigate()
  return (
    <AdminUsersPage
      status={status}
      cursor={cursor}
      onFilter={(status) => {
        void navigate({ search: { status } })
      }}
      onPage={(cursor) => {
        void navigate({ search: { status, cursor } })
      }}
    />
  )
}
