import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react"
import { Terminal } from "@xterm/xterm"
import { FitAddon } from "@xterm/addon-fit"

import { useSession } from "../../lib/auth/session-context"
import { openTerminalSocket, requestAttachment } from "./api"
import { TerminalController } from "./terminal-controller"
import "@xterm/xterm/css/xterm.css"
import "./terminal.css"

export default function TerminalSession() {
  const { controller: session } = useSession()
  const host = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const controls = useRef<HTMLDivElement>(null)
  const controlNextKey = useRef(false)
  const [controlPressed, setControlPressed] = useState(false)
  const [screenReader, setScreenReader] = useState(false)
  const controller = useMemo(
    () =>
      new TerminalController({
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
    [session]
  )
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot
  )

  useEffect(() => {
    if (!host.current) return
    const widget = new Terminal({
      cursorBlink: true,
      scrollback: 10_000,
      fontSize: 14,
      fontFamily: '"Cascadia Code", "Consolas", monospace',
      theme: {
        background: "#15171c",
        foreground: "#edf0f5",
        cursor: "#edf0f5",
      },
      allowProposedApi: false,
    })
    const fit = new FitAddon()
    widget.loadAddon(fit)
    widget.open(host.current)
    terminal.current = widget
    widget.textarea?.setAttribute(
      "aria-label",
      "Private terminal. Press Control Shift M to return to terminal controls."
    )
    widget.attachCustomKeyEventHandler((event) => {
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
      if (host.current?.clientWidth && host.current.clientHeight) fit.fit()
    }
    const observer = new ResizeObserver(fitView)
    observer.observe(host.current)
    window.visualViewport?.addEventListener("resize", fitView)
    fitView()
    // Opening replaces its button. Keep keyboard users in the new terminal,
    // unless they moved focus elsewhere while the lazy bundle was loading.
    if (document.activeElement === document.body) widget.focus()
    void controller.connect()
    return () => {
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
    if (terminal.current)
      terminal.current.options.screenReaderMode = screenReader
  }, [screenReader])

  const connected = state.status === "connected"
  const opening = state.status === "opening"
  function open() {
    terminal.current?.focus()
    void controller.connect()
  }
  function takeOver() {
    terminal.current?.focus()
    void controller.connect(true)
  }
  function disconnect() {
    controls.current?.focus()
    controller.disconnect()
  }
  function key(value: string) {
    controller.inputText(value)
    terminal.current?.focus()
  }

  return (
    <div className="terminal-panel">
      <div
        ref={controls}
        tabIndex={-1}
        className="terminal-actions"
        aria-label="Terminal actions"
      >
        <button type="button" onClick={open} disabled={opening || connected}>
          Reconnect
        </button>
        <button
          type="button"
          onClick={disconnect}
          disabled={!connected && !opening}
        >
          Disconnect display
        </button>
        {state.status === "conflict" && (
          <button type="button" onClick={takeOver}>
            Take over terminal
          </button>
        )}
        <label>
          <input
            type="checkbox"
            checked={screenReader}
            onChange={(event) => setScreenReader(event.target.checked)}
          />{" "}
          Screen reader mode
        </label>
      </div>
      <p className="terminal-status" role="status" aria-live="polite">
        {state.message}
      </p>
      <div
        ref={host}
        className="terminal-screen"
        aria-label="Terminal display"
      />
      <div className="terminal-keys" aria-label="Terminal keyboard shortcuts">
        <button
          type="button"
          disabled={!connected}
          aria-pressed={controlPressed}
          onClick={() => {
            controlNextKey.current = !controlNextKey.current
            setControlPressed(controlNextKey.current)
            terminal.current?.focus()
          }}
        >
          Ctrl
        </button>
        <button type="button" disabled={!connected} onClick={() => key("\r")}>
          Enter
        </button>
        <button type="button" disabled={!connected} onClick={() => key("\t")}>
          Tab
        </button>
        <button
          type="button"
          disabled={!connected}
          onClick={() => key("\u0003")}
        >
          Ctrl+C
        </button>
        <button
          type="button"
          disabled={!connected}
          onClick={() => key("\u0004")}
        >
          Ctrl+D
        </button>
        <button
          type="button"
          disabled={!connected}
          onClick={() => key("\u001b")}
        >
          Esc
        </button>
      </div>
      <p className="terminal-help">
        Press Ctrl+Shift+M to leave terminal input and reach these controls.
        Closing Factory keeps the shell running; stopping the environment ends
        its processes.
      </p>
    </div>
  )
}
