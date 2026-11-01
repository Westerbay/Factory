import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react"
import { Terminal } from "@xterm/xterm"
import { FitAddon } from "@xterm/addon-fit"
import { Maximize2, Minimize2, Settings2, TerminalSquare } from "lucide-react"

import { Button, IconButton } from "../../components/ui/button"
import { useTheme } from "../../lib/theme/theme"
import { useSession } from "../../lib/auth/session-context"
import { openTerminalSocket, requestAttachment } from "./api"
import { TerminalController } from "./terminal-controller"
import "@xterm/xterm/css/xterm.css"
import "./terminal.css"

export default function TerminalSession({
  conversationId,
  projectName,
  onOwnDetach,
  consumeRecentOwnDetach,
  expanded,
  onExpandedChange,
}: {
  conversationId?: string
  projectName?: string
  onOwnDetach: () => void
  consumeRecentOwnDetach: () => boolean
  expanded: boolean
  onExpandedChange: (expanded: boolean) => void
}) {
  const locale = useLocale()
  const { controller: session } = useSession()
  const theme = useTheme()
  const host = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const controls = useRef<HTMLDivElement>(null)
  const expandToggle = useRef<HTMLButtonElement>(null)
  const expansion = useRef({ expanded, onExpandedChange })
  expansion.current = { expanded, onExpandedChange }
  const controlNextKey = useRef(false)
  const [controlPressed, setControlPressed] = useState(false)
  const [screenReader, setScreenReader] = useState(false)
  const controller = useMemo(
    () =>
      new TerminalController({
        conversationId,
        onOwnDetach,
        consumeRecentOwnDetach,
        session,
        request: requestAttachment,
        openSocket: openTerminalSocket,
        display: {
          write: (data, done) => terminal.current?.write(data, done),
          reset: () => {
            controlNextKey.current = false
            setControlPressed(false)
            terminal.current?.reset()
          },
        },
      }),
    // The panel owns stable state; callbacks refer to the current terminal key.
    [session, conversationId]
  )
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
  )

  useEffect(() => {
    if (!host.current) return
    const element = host.current
    const style = getComputedStyle(element)
    let disposed = false
    const widget = new Terminal({
      cursorBlink: true,
      scrollback: 10_000,
      fontSize: 14,
      minimumContrastRatio: 4.5,
      fontFamily: style.fontFamily,
      theme: {
        background: style.backgroundColor,
        foreground: style.color,
        cursor: style.color,
        scrollbarSliderBackground: style
          .getPropertyValue("--scrollbar-thumb")
          .trim(),
        scrollbarSliderHoverBackground: style
          .getPropertyValue("--scrollbar-thumb-hover")
          .trim(),
        scrollbarSliderActiveBackground: style
          .getPropertyValue("--scrollbar-thumb-active")
          .trim(),
      },
      allowProposedApi: false,
    })
    const fit = new FitAddon()
    widget.loadAddon(fit)
    terminal.current = widget
    widget.attachCustomKeyEventHandler((event) => {
      if (event.key === "Escape" && expansion.current.expanded) {
        if (event.type === "keydown") {
          event.preventDefault()
          expansion.current.onExpandedChange(false)
          expandToggle.current?.focus({ preventScroll: true })
        }
        return false
      }
      if (event.ctrlKey && event.shiftKey && event.code === "KeyM") {
        if (event.type === "keydown") controls.current?.focus()
        return false
      }
      return true
    })
    const input = widget.onData((data) => {
      if (controlNextKey.current) {
        controlNextKey.current = false
        setControlPressed(false)
        const code = data.charCodeAt(0)
        if (
          data.length === 1 &&
          ((code >= 64 && code <= 95) || (code >= 97 && code <= 122))
        ) {
          data = String.fromCharCode(code & 31)
        }
      }
      controller.inputText(data)
    })
    const resized = widget.onResize(({ cols, rows }) =>
      controller.resize(cols, rows)
    )
    const fitView = () => {
      if (!disposed && element.clientWidth && element.clientHeight) fit.fit()
    }
    const observer = new ResizeObserver(fitView)
    const attach = () => {
      if (disposed) return
      widget.open(element)
      widget.textarea?.setAttribute(
        "aria-label",
        m.private_terminal_press_control_shift_m_to_return_to_terminal_cont()
      )
      observer.observe(element)
      window.visualViewport?.addEventListener("resize", fitView)
      fitView()
      void controller.connect()
    }
    // Measure the actual code font on first open. Failed font downloads still
    // leave the system monospace fallback usable.
    void document.fonts
      .load("14px " + style.fontFamily)
      .then(() => document.fonts.ready.then(attach), attach)
    return () => {
      disposed = true
      observer.disconnect()
      window.visualViewport?.removeEventListener("resize", fitView)
      controller.dispose()
      input.dispose()
      resized.dispose()
      widget.dispose()
      terminal.current = null
    }
  }, [controller])

  useEffect(() => {
    if (!host.current || !terminal.current) return
    const style = getComputedStyle(host.current)
    terminal.current.options.theme = {
      background: style.backgroundColor,
      foreground: style.color,
      cursor: style.color,
      scrollbarSliderBackground: style
        .getPropertyValue("--scrollbar-thumb")
        .trim(),
      scrollbarSliderHoverBackground: style
        .getPropertyValue("--scrollbar-thumb-hover")
        .trim(),
      scrollbarSliderActiveBackground: style
        .getPropertyValue("--scrollbar-thumb-active")
        .trim(),
    }
  }, [theme])

  useEffect(() => {
    terminal.current?.textarea?.setAttribute(
      "aria-label",
      m.private_terminal_press_control_shift_m_to_return_to_terminal_cont()
    )
  }, [locale])

  useEffect(() => {
    if (terminal.current)
      terminal.current.options.screenReaderMode = screenReader
  }, [screenReader])

  const connected = state.status === "connected"
  function open() {
    terminal.current?.focus()
    void controller.connect()
  }
  function takeOver() {
    terminal.current?.focus()
    void controller.connect(true)
  }
  function key(value: string) {
    controller.inputText(value)
    terminal.current?.focus()
  }

  return (
    <div
      className="terminal-panel"
      onKeyDown={(event) => {
        if (event.key !== "Escape" || !expanded || event.defaultPrevented)
          return
        event.preventDefault()
        onExpandedChange(false)
        expandToggle.current?.focus({ preventScroll: true })
      }}
    >
      <div
        ref={controls}
        tabIndex={-1}
        className="terminal-header"
        role="group"
        aria-label={m.terminal_controls()}
      >
        <h2>
          <TerminalSquare size={15} aria-hidden="true" />
          Terminal{projectName ? ` · ${projectName}` : ""}
        </h2>
        <div className="terminal-header-actions">
          <IconButton
            ref={expandToggle}
            label={expanded ? m.restore_terminal_size() : m.maximize_terminal()}
            aria-pressed={expanded}
            aria-keyshortcuts={expanded ? "Escape" : undefined}
            onClick={() => onExpandedChange(!expanded)}
          >
            {expanded ? (
              <Minimize2 aria-hidden="true" size={16} />
            ) : (
              <Maximize2 aria-hidden="true" size={16} />
            )}
          </IconButton>
          <details
            className="terminal-settings"
            onKeyDown={(event) => {
              if (event.key !== "Escape" || !event.currentTarget.open) return
              event.preventDefault()
              event.currentTarget.open = false
              event.currentTarget.querySelector("summary")?.focus()
            }}
          >
            <summary
              aria-label={m.terminal_settings()}
              title={m.terminal_settings()}
            >
              <Settings2 size={16} aria-hidden="true" />
            </summary>
            <div className="terminal-settings-content">
              <label>
                <input
                  type="checkbox"
                  checked={screenReader}
                  onChange={(event) => setScreenReader(event.target.checked)}
                />
                {m.screen_reader_mode()}{" "}
              </label>
              <p>
                <kbd>Ctrl+Shift+M</kbd> {m.returns_to_terminal_controls()}
              </p>
            </div>
          </details>
        </div>
      </div>
      <p
        className={state.message ? "terminal-status" : "sr-only"}
        role="status"
        aria-live="polite"
      >
        {(state.message && translateMessage(state.message)) ||
          (connected ? m.terminal_connected() : m.loading_terminal())}
      </p>
      {(state.status === "error" || state.status === "disconnected") && (
        <div className="terminal-recovery">
          <Button onClick={open}>{m.reconnect()}</Button>
        </div>
      )}
      {state.status === "conflict" && (
        <div className="terminal-recovery">
          <Button onClick={takeOver}>{m.take_over_terminal()}</Button>
        </div>
      )}
      <div
        ref={host}
        className="terminal-screen"
        aria-label={m.terminal_display()}
      />
      <div
        className="terminal-keys"
        aria-label={m.terminal_keyboard_shortcuts()}
      >
        <Button
          variant="ghost"
          disabled={!connected}
          aria-pressed={controlPressed}
          onClick={() => {
            controlNextKey.current = !controlNextKey.current
            setControlPressed(controlNextKey.current)
            terminal.current?.focus()
          }}
        >
          Ctrl
        </Button>
        <Button variant="ghost" disabled={!connected} onClick={() => key("\r")}>
          Enter
        </Button>
        <Button variant="ghost" disabled={!connected} onClick={() => key("\t")}>
          Tab
        </Button>
        <Button
          variant="ghost"
          disabled={!connected}
          onClick={() => key("\u0003")}
        >
          Ctrl+C
        </Button>
        <Button
          variant="ghost"
          disabled={!connected}
          onClick={() => key("\u0004")}
        >
          Ctrl+D
        </Button>
        <Button
          variant="ghost"
          disabled={!connected}
          onClick={() => key("\u001b")}
        >
          Esc
        </Button>
      </div>
    </div>
  )
}
