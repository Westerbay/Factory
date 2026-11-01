import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useEffect, useRef, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"

import { Button } from "../../components/ui/button"
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
  useLocale()
  const { state } = useSession()
  if (state.status !== "active") return null
  if (state.session.user.role !== "admin")
    return (
      <section>
        <h1>{m.administrator_access_required()}</h1>
        <p>{m.your_account_cannot_manage_other_users()}</p>
        <Link to="/app">{m.return_to_your_workspace()}</Link>
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
  useLocale()
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
      <h1 id="users-title">{m.user_access()}</h1>
      <p className="introduction">
        {m.review_requests_and_manage_access_to_factory_activating_an_accoun()}{" "}
      </p>
      <div className="admin-toolbar">
        <div
          className="admin-filters"
          role="group"
          aria-label={m.filter_users_by_access_status()}
        >
          {(["pending", "active", "suspended"] as const).map((filter) => (
            <Button
              key={filter}
              type="button"
              aria-pressed={status === filter}
              onClick={() => {
                setNotice(undefined)
                onFilter(filter)
              }}
            >
              {translateMessage(
                filter.charAt(0).toUpperCase() + filter.slice(1)
              )}
            </Button>
          ))}
        </div>
        <Button
          type="button"
          disabled={users.isFetching}
          onClick={() => void users.refetch()}
        >
          {m.refresh_list()}{" "}
        </Button>
      </div>
      {notice && (
        <p className="admin-notice" role="status" ref={noticeRef} tabIndex={-1}>
          {translateMessage(notice.message)}
        </p>
      )}
      {users.isPending ? (
        <p role="status">{m.loading_users()}</p>
      ) : users.isError ? (
        <p className="admin-notice" role="alert">
          {m.the_user_list_could_not_be_loaded_use_refresh_list_to_try_again()}{" "}
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
            <p className="admin-empty">
              {m.no_status_users_on_this_page({
                status: translateMessage(
                  status.charAt(0).toUpperCase() + status.slice(1)
                ),
              })}
            </p>
          )}
          <nav className="admin-pagination" aria-label={m.user_list_pages()}>
            <Button
              type="button"
              disabled={!cursor || users.isFetching}
              onClick={() => onPage()}
            >
              {m.first_page()}{" "}
            </Button>
            <span>
              {m.count_shown_up_to_20_per_page({
                count: users.data.items.length,
              })}
            </span>
            <Button
              type="button"
              disabled={!users.data.nextCursor || users.isFetching}
              onClick={() => {
                if (users.data.nextCursor) onPage(users.data.nextCursor)
              }}
            >
              {m.next_page()}{" "}
            </Button>
          </nav>
        </>
      )}
    </section>
  )
}
