import { useRef, useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"

import { Button } from "../../components/ui/button"
import { ApiFailure } from "../../lib/api/client"
import { changeAccess, type AdminUser, type AccessAction } from "./api"
import { SuspendDialog } from "./suspend-dialog"

export function UserRow({
  user,
  csrfToken,
  onNotice,
}: {
  user: AdminUser
  csrfToken: string
  onNotice: (message: string, focus?: boolean) => void
}) {
  const [confirming, setConfirming] = useState(false)
  const suspendButton = useRef<HTMLButtonElement>(null)
  const queryClient = useQueryClient()
  const change = useMutation({
    mutationFn: (status: AccessAction) => changeAccess(user, status, csrfToken),
    onMutate: () => onNotice("Saving access…", true),
    onSuccess: async (_data, status) => {
      onNotice(
        status === "active"
          ? "Access activated. The member must sign in again."
          : "Access suspended and sessions revoked."
      )
      await queryClient.invalidateQueries({ queryKey: ["admin-users"] })
    },
    onError: async (error) => {
      if (error instanceof ApiFailure && error.status === 409) {
        onNotice(
          "This account changed or cannot be edited. The list has been refreshed; review it before trying again."
        )
        await queryClient.invalidateQueries({ queryKey: ["admin-users"] })
      } else {
        onNotice(
          "The access change could not be confirmed. Refresh the list before trying again."
        )
      }
    },
  })
  const name = user.displayName || user.handle

  return (
    <li className="admin-user-card" aria-busy={change.isPending}>
      <div className="admin-identity">
        <h2>{name}</h2>
        <p>@{user.handle}</p>
      </div>
      <dl className="admin-details">
        <div>
          <dt>Provider</dt>
          <dd>{user.provider}</dd>
        </div>
        <div>
          <dt>Role</dt>
          <dd>{user.role}</dd>
        </div>
        <div>
          <dt>Status</dt>
          <dd>{user.status}</dd>
        </div>
      </dl>
      <div className="admin-actions">
        {user.role === "admin" ? (
          <span className="admin-protected">Protected administrator</span>
        ) : (
          <>
            {user.status !== "active" && (
              <Button
                type="button"
                disabled={change.isPending}
                onClick={() => change.mutate("active")}
                aria-label={`Activate ${name}`}
              >
                Activate
              </Button>
            )}
            {user.status !== "suspended" && (
              <Button
                ref={suspendButton}
                type="button"
                disabled={change.isPending}
                onClick={() => setConfirming(true)}
                aria-label={`Suspend ${name}`}
              >
                Suspend
              </Button>
            )}
          </>
        )}
        {change.isPending && <span role="status">Saving access…</span>}
      </div>
      {confirming && (
        <SuspendDialog
          name={name}
          onCancel={() => {
            setConfirming(false)
            requestAnimationFrame(() => suspendButton.current?.focus())
          }}
          onConfirm={() => {
            setConfirming(false)
            change.mutate("suspended")
          }}
        />
      )}
    </li>
  )
}
