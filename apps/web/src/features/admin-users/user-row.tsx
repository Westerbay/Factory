import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
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
  useLocale()
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
          <dt>{m.provider()}</dt>
          <dd>{user.provider}</dd>
        </div>
        <div>
          <dt>{m.role()}</dt>
          <dd>{translateMessage(user.role)}</dd>
        </div>
        <div>
          <dt>{m.status()}</dt>
          <dd>{translateMessage(user.status)}</dd>
        </div>
      </dl>
      <div className="admin-actions">
        {user.role === "admin" ? (
          <span className="admin-protected">{m.protected_administrator()}</span>
        ) : (
          <>
            {user.status !== "active" && (
              <Button
                type="button"
                disabled={change.isPending}
                onClick={() => change.mutate("active")}
                aria-label={m.activate_name({ name })}
              >
                {m.activate()}{" "}
              </Button>
            )}
            {user.status !== "suspended" && (
              <Button
                ref={suspendButton}
                type="button"
                disabled={change.isPending}
                onClick={() => setConfirming(true)}
                aria-label={m.suspend_name({ name })}
              >
                {m.suspend()}{" "}
              </Button>
            )}
          </>
        )}
        {change.isPending && <span role="status">{m.saving_access()}</span>}
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
