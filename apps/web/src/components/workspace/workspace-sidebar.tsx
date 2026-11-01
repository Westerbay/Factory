import { useTranslation } from "react-i18next"
import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"

import { MIN_SPLIT_WIDTH, PRIMARY_MIN_WIDTH } from "../ui/split-pane"

const DEFAULT_WIDTH = 280
const MIN_WIDTH = 224
const MAX_WIDTH = 480
const SEPARATOR_WIDTH = 1

function subscribeViewport(listener: () => void) {
  window.addEventListener("resize", listener)
  return () => window.removeEventListener("resize", listener)
}

function viewportWidth() {
  return window.innerWidth
}

/** Desktop navigation retains its requested width while hidden or in compact mode. */
export function WorkspaceSidebar({
  children,
  visible,
  compact,
  terminalVisible,
  onCollapse,
}: {
  children: ReactNode
  visible: boolean
  compact: boolean
  terminalVisible: boolean
  onCollapse: () => void
}) {
  const { t: translate } = useTranslation()
  const viewport = useSyncExternalStore(subscribeViewport, viewportWidth)
  const [requestedWidth, setRequestedWidth] = useState(DEFAULT_WIDTH)
  const [resizing, setResizing] = useState(false)
  const drag = useRef<{
    pointerId: number
    target: HTMLDivElement
    startX: number
    startWidth: number
  } | null>(null)
  const minimumBody = terminalVisible ? MIN_SPLIT_WIDTH : PRIMARY_MIN_WIDTH
  const maximum = Math.max(
    MIN_WIDTH,
    Math.min(MAX_WIDTH, viewport - minimumBody - SEPARATOR_WIDTH)
  )
  const width = Math.min(maximum, Math.max(MIN_WIDTH, requestedWidth))
  const split = !compact && visible

  useEffect(() => {
    if (split) return
    const current = drag.current
    drag.current = null
    if (current?.target.hasPointerCapture(current.pointerId))
      current.target.releasePointerCapture(current.pointerId)
    setResizing(false)
  }, [split])

  function resize(next: number) {
    setRequestedWidth(Math.min(maximum, Math.max(MIN_WIDTH, next)))
  }

  if (compact) return null
  return (
    <>
      <div
        className="workspace-sidebar-track t-resize"
        data-collapsed={!visible ? "" : undefined}
        data-resizing={resizing ? "" : undefined}
        style={{ width: visible ? width : 0 }}
        inert={!visible}
        aria-hidden={!visible}
      >
        <aside
          id="workspace-sidebar"
          className="workspace-sidebar"
          style={{ width }}
          aria-label={translate("Workspace navigation")}
        >
          {children}
        </aside>
      </div>
      {split && (
        <div
          className="split-pane-separator workspace-sidebar-separator"
          data-resizing={resizing ? "" : undefined}
          role="separator"
          aria-label={translate("Chats sidebar size")}
          aria-controls="workspace-sidebar"
          aria-orientation="vertical"
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={maximum}
          aria-valuenow={width}
          aria-valuetext={`${width} pixels`}
          tabIndex={0}
          onKeyDown={(event) => {
            const step = event.shiftKey ? 48 : 16
            switch (event.key) {
              case "ArrowLeft":
                resize(width - step)
                break
              case "ArrowRight":
                resize(width + step)
                break
              case "Home":
                resize(MIN_WIDTH)
                break
              case "End":
                resize(maximum)
                break
              case "Enter":
                onCollapse()
                break
              default:
                return
            }
            event.preventDefault()
          }}
          onPointerDown={(event) => {
            if (event.button !== 0 || !event.isPrimary) return
            event.preventDefault()
            event.currentTarget.focus({ preventScroll: true })
            event.currentTarget.setPointerCapture(event.pointerId)
            drag.current = {
              pointerId: event.pointerId,
              target: event.currentTarget,
              startX: event.clientX,
              startWidth: width,
            }
            setResizing(true)
          }}
          onPointerMove={(event) => {
            const current = drag.current
            if (!current || current.pointerId !== event.pointerId) return
            resize(current.startWidth + event.clientX - current.startX)
          }}
          onPointerUp={(event) => {
            if (drag.current?.pointerId !== event.pointerId) return
            drag.current = null
            setResizing(false)
            if (event.currentTarget.hasPointerCapture(event.pointerId))
              event.currentTarget.releasePointerCapture(event.pointerId)
          }}
          onPointerCancel={() => {
            drag.current = null
            setResizing(false)
          }}
          onLostPointerCapture={() => {
            drag.current = null
            setResizing(false)
          }}
        />
      )}
    </>
  )
}
