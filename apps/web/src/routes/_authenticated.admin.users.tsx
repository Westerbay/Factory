import { createRoute } from "@tanstack/react-router"

import { AppHeader } from "../components/app-header"
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
    <div className="admin-shell">
      <AppHeader />
      <div className="admin-content">
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
      </div>
    </div>
  )
}
