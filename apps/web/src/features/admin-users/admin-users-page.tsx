import { useEffect, useRef, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"

import { useSession } from "../../lib/auth/session-context"
import type { Session } from "../../lib/auth/session-api"
import { listUsers, type UserStatus } from "./api"
import { UserRow } from "./user-row"
import "./admin-users.css"

type Props = {
  status: UserStatus
  cursor?: string
  onFilter: (status: UserStatus) => void
  onPage: (cursor?: string) => void
}

export function AdminUsersPage(props: Props) {
  const { state } = useSession()
  if (state.status !== "active") return null
  if (state.session.user.role !== "admin")
    return (
      <section>
        <h1>Administrator access required</h1>
        <p>Your account cannot manage other users.</p>
        <Link to="/app">Return to your workspace</Link>
      </section>
    )
  return <AdminUsersList {...props} session={state.session} />
}

function AdminUsersList({
  status,
  cursor,
  onFilter,
  onPage,
  session,
}: Props & { session: Session }) {
  const [notice, setNotice] = useState<{ message: string; focus: boolean }>()
  const noticeRef = useRef<HTMLParagraphElement>(null)
  useEffect(() => {
    if (notice?.focus) noticeRef.current?.focus()
  }, [notice])
  function showNotice(message: string, focus = false) {
    setNotice({ message, focus })
  }
  const users = useQuery({
    queryKey: ["admin-users", session.user.id, status, cursor],
    queryFn: ({ signal }) => listUsers(status, cursor, signal),
  })
  return (
    <section className="admin-users" aria-labelledby="users-title">
      <p className="eyebrow">Administration</p>
      <h1 id="users-title">User access</h1>
      <p className="introduction">
        Review requests and manage access to Factory. Activating an account
        requires the member to sign in again.
      </p>
      <div className="admin-toolbar">
        <div
          className="admin-filters"
          role="group"
          aria-label="Filter users by access status"
        >
          {(["pending", "active", "suspended"] as const).map((filter) => (
            <button
              key={filter}
              type="button"
              aria-pressed={status === filter}
              onClick={() => {
                setNotice(undefined)
                onFilter(filter)
              }}
            >
              {filter.charAt(0).toUpperCase() + filter.slice(1)}
            </button>
          ))}
        </div>
        <button
          type="button"
          disabled={users.isFetching}
          onClick={() => void users.refetch()}
        >
          Refresh list
        </button>
      </div>
      {notice && (
        <p className="admin-notice" role="status" ref={noticeRef} tabIndex={-1}>
          {notice.message}
        </p>
      )}
      {users.isPending ? (
        <p role="status">Loading users…</p>
      ) : users.isError ? (
        <p className="admin-notice" role="alert">
          The user list could not be loaded. Use Refresh list to try again.
        </p>
      ) : (
        <>
          {users.data.items.length ? (
            <ul className="admin-user-list">
              {users.data.items.map((user) => (
                <UserRow
                  key={user.id}
                  user={user}
                  csrfToken={session.csrfToken}
                  onNotice={showNotice}
                />
              ))}
            </ul>
          ) : (
            <p className="admin-empty">No {status} users on this page.</p>
          )}
          <nav className="admin-pagination" aria-label="User list pages">
            <button
              type="button"
              disabled={!cursor || users.isFetching}
              onClick={() => onPage()}
            >
              First page
            </button>
            <span>{users.data.items.length} shown · Up to 20 per page</span>
            <button
              type="button"
              disabled={!users.data.nextCursor || users.isFetching}
              onClick={() => {
                if (users.data.nextCursor) onPage(users.data.nextCursor)
              }}
            >
              Next page
            </button>
          </nav>
        </>
      )}
    </section>
  )
}
