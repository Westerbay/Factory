import { useEffect, useId, useRef, type ReactNode, type RefObject } from "react"

export function Modal({
  open,
  onClose,
  title,
  children,
  returnFocusRef,
  descriptionId,
  className,
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  returnFocusRef?: RefObject<HTMLElement | null>
  descriptionId?: string
  className?: string
}) {
  const ref = useRef<HTMLDialogElement>(null)
  const controlledCloses = useRef(0)
  const returnFocus = useRef(returnFocusRef)
  const titleId = useId()

  useEffect(() => {
    returnFocus.current = returnFocusRef
  }, [returnFocusRef])

  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) {
      dialog.showModal()
      const initialFocus =
        dialog.querySelector<HTMLElement>("[data-autofocus]") ??
        dialog.querySelector<HTMLButtonElement>("button:not(:disabled)")
      initialFocus?.focus()
    } else if (!open && dialog.open) {
      // Native close restores the opener without overriding focus that a caller
      // intentionally moved to another result or route.
      controlledCloses.current++
      dialog.close()
      returnFocus.current?.current?.focus({ preventScroll: true })
    }
  }, [open])

  useEffect(() => {
    const dialog = ref.current
    return () => {
      if (dialog?.open) {
        controlledCloses.current++
        dialog.close()
        returnFocus.current?.current?.focus({ preventScroll: true })
      }
    }
  }, [])

  return (
    <dialog
      ref={ref}
      className={["ui-modal", className].filter(Boolean).join(" ")}
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        // Nested dialogs must handle their own Escape without closing this one.
        if (event.target !== event.currentTarget) return
        event.preventDefault()
        onClose()
      }}
      onClose={(event) => {
        // React bubbles close from a nested dialog through this component.
        if (event.target !== event.currentTarget) return
        if (controlledCloses.current > 0) {
          controlledCloses.current--
          return
        }
        if (open) onClose()
      }}
    >
      <h2 id={titleId}>{title}</h2>
      {children}
    </dialog>
  )
}
