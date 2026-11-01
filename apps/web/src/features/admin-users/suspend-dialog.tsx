import { useTranslation } from "react-i18next"
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
  const { t: translate } = useTranslation()
  const descriptionId = useId()

  return (
    <Modal
      open
      onClose={onCancel}
      title={translate("Suspend {{name}}?", { name })}
      descriptionId={descriptionId}
    >
      <p id={descriptionId}>
        {translate(
          "All of this user's sessions will be revoked immediately. They will not be able to sign in until you reactivate their access."
        )}{" "}
      </p>
      <div className="ui-modal-actions">
        <Button data-autofocus onClick={onCancel}>
          {translate("Cancel")}{" "}
        </Button>
        <Button variant="danger" onClick={onConfirm}>
          {translate("Suspend access")}{" "}
        </Button>
      </div>
    </Modal>
  )
}
