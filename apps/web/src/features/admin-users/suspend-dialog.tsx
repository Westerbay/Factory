import { useId } from "react"

import { Button } from "../../components/ui/button"
import { Modal } from "../../components/ui/modal"

export function SuspendDialog({
  name,
  onCancel,
  onConfirm,
}: {
  name: string
  onCancel: () => void
  onConfirm: () => void
}) {
  const descriptionId = useId()

  return (
    <Modal
      open
      onClose={onCancel}
      title={`Suspend ${name}?`}
      descriptionId={descriptionId}
    >
      <p id={descriptionId}>
        All of this user's sessions will be revoked immediately. They will not
        be able to sign in until you reactivate their access.
      </p>
      <div className="ui-modal-actions">
        <Button data-autofocus onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="danger" onClick={onConfirm}>
          Suspend access
        </Button>
      </div>
    </Modal>
  )
}
