import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import { useEffect, useRef, useState, type ReactNode } from "react"
import "./split-pane.css"

const SEPARATOR_WIDTH = 1
export const PRIMARY_MIN_WIDTH = 320
const SECONDARY_MIN_WIDTH = 280
export const MIN_SPLIT_WIDTH =
  PRIMARY_MIN_WIDTH + SECONDARY_MIN_WIDTH + SEPARATOR_WIDTH
const MIN_PERCENT = 25
const MAX_PERCENT = 65

export function SplitPane({
  primary,
  secondary,
  secondaryVisible,
  secondaryExpanded = false,
  onSecondaryVisibleChange,
  compact,
  secondaryId,
  defaultSecondaryPercent = 40,
}: {
  primary: ReactNode
  secondary: ReactNode
  secondaryVisible: boolean
  secondaryExpanded?: boolean
  onSecondaryVisibleChange: (visible: boolean) => void
  compact: boolean
  secondaryId: string
  defaultSecondaryPercent?: number
}) {
  useLocale()
  const host = useRef<HTMLDivElement>(null)
  const primaryPane = useRef<HTMLDivElement>(null)
  const drag = useRef<{
    pointerId: number
    target: HTMLDivElement
    startX: number
    startPercent: number
    width: number
  } | null>(null)
  const [width, setWidth] = useState(0)
  const [requestedPercent, setRequestedPercent] = useState<number>()
  const [resizing, setResizing] = useState(false)
  const primaryHidden = secondaryExpanded || (compact && secondaryVisible)
  const split = !compact && secondaryVisible && !secondaryExpanded
  const availableWidth = Math.max(0, width - SEPARATOR_WIDTH)
  const canFitMinimums = width >= MIN_SPLIT_WIDTH
  const minimum = canFitMinimums
    ? Math.max(MIN_PERCENT, (SECONDARY_MIN_WIDTH / availableWidth) * 100)
    : MIN_PERCENT
  const maximum = canFitMinimums
    ? Math.min(MAX_PERCENT, 100 - (PRIMARY_MIN_WIDTH / availableWidth) * 100)
    : MAX_PERCENT
  const percent = Math.min(
    maximum,
    Math.max(minimum, requestedPercent ?? defaultSecondaryPercent)
  )

  useEffect(() => {
    const element = host.current
    if (!element) return
    setWidth(element.clientWidth)
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(entry.contentRect.width)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (split) return
    const current = drag.current
    drag.current = null
    if (current?.target.hasPointerCapture(current.pointerId))
      current.target.releasePointerCapture(current.pointerId)
    setResizing(false)
  }, [split])

  function resize(next: number) {
    setRequestedPercent(Math.min(maximum, Math.max(minimum, next)))
  }

  function collapse() {
    const toggle = document.querySelector<HTMLElement>(
      '[aria-controls="' +
        CSS.escape(secondaryId) +
        '"]:not([role="separator"])'
    )
    const focusTarget = toggle ?? primaryPane.current
    focusTarget?.focus({ preventScroll: true })
    onSecondaryVisibleChange(false)
  }

  return (
    <div
      ref={host}
      className="split-pane"
      data-resizing={split && resizing ? "" : undefined}
      style={{
        gridTemplateColumns: primaryHidden
          ? "minmax(0, 0fr) 0px minmax(0, 100fr)"
          : split
            ? `minmax(0, ${100 - percent}fr) ${SEPARATOR_WIDTH}px minmax(0, ${percent}fr)`
            : "minmax(0, 100fr) 0px minmax(0, 0fr)",
      }}
    >
      <div
        ref={primaryPane}
        className="split-pane-primary"
        data-collapsed={primaryHidden ? "" : undefined}
        inert={primaryHidden}
        aria-hidden={primaryHidden}
        tabIndex={-1}
      >
        {primary}
      </div>
      {split && (
        <div
          className="split-pane-separator"
          role="separator"
          aria-label={m.side_panel_size()}
          aria-controls={secondaryId}
          aria-orientation="vertical"
          aria-valuemin={Number(minimum.toFixed(2))}
          aria-valuemax={Number(maximum.toFixed(2))}
          aria-valuenow={Number(percent.toFixed(2))}
          tabIndex={0}
          onKeyDown={(event) => {
            const step = event.shiftKey ? 10 : 2
            switch (event.key) {
              case "ArrowLeft":
                resize(percent + step)
                break
              case "ArrowRight":
                resize(percent - step)
                break
              case "Home":
                resize(minimum)
                break
              case "End":
                resize(maximum)
                break
              case "Enter":
                collapse()
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
              startPercent: percent,
              width: availableWidth,
            }
            setResizing(true)
          }}
          onPointerMove={(event) => {
            const current = drag.current
            if (
              !current ||
              current.pointerId !== event.pointerId ||
              !current.width
            )
              return
            resize(
              current.startPercent +
                ((current.startX - event.clientX) / current.width) * 100
            )
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
      <div
        id={secondaryId}
        className="split-pane-secondary"
        hidden={!secondaryVisible}
      >
        {secondary}
      </div>
    </div>
  )
}
