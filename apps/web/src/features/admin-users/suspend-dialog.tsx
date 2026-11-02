import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
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
  useLocale()
  const descriptionId = useId()

  return (
    <Modal
      open
      onClose={onCancel}
      title={m.suspend_name_2391b905({ name })}
      descriptionId={descriptionId}
    >
      <p id={descriptionId}>
        {m.all_of_this_user_s_sessions_will_be_revoked_immediately_they_will()}{" "}
      </p>
      <div className="ui-modal-actions">
        <Button data-autofocus onClick={onCancel}>
          {m.cancel()}{" "}
        </Button>
        <Button variant="danger" onClick={onConfirm}>
          {m.suspend_access()}{" "}
        </Button>
      </div>
    </Modal>
  )
}
