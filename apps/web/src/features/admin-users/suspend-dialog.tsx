import { useEffect, useRef } from "react"

export function SuspendDialog({
  name,
  onCancel,
  onConfirm,
}: {
  name: string
  onCancel: () => void
  onConfirm: () => void
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    dialog?.showModal()
    return () => {
      dialog?.close()
    }
  }, [])

  return (
    <dialog
      className="suspend-dialog"
      ref={ref}
      onCancel={onCancel}
      aria-labelledby="suspend-title"
      aria-describedby="suspend-description"
    >
      <h2 id="suspend-title">Suspend {name}?</h2>
      <p id="suspend-description">
        All of this user's sessions will be revoked immediately. They will not
        be able to sign in until you reactivate their access.
      </p>
      <div className="admin-actions">
        <button type="button" autoFocus onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="danger-action" onClick={onConfirm}>
          Suspend access
        </button>
      </div>
    </dialog>
  )
}
