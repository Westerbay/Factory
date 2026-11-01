import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"
import {
  PanelLeft,
  PanelLeftClose,
  PanelLeftOpen,
  Terminal,
  X,
} from "lucide-react"

import { ConnectedProvider } from "../../features/agent-connections/connected-provider"
import { ClaudeSettings } from "../../features/agent-connections/claude-connection"
import { ClaudeLogin } from "../../features/agent-connections/claude-login"
import { useClaudeConnection } from "../../features/agent-connections/use-claude-connection"
import { ProviderConnection } from "../../features/agent-connections/provider-connection"
import { useConnection } from "../../features/agent-connections/use-connection"
import { listRecoverable } from "../../features/conversations/api"
import { Chat } from "../../features/conversations/chat"
import { canStopEnvironment } from "../../features/environments/environment-controller"
import { useEnvironment } from "../../features/environments/use-environment"
import { TerminalPanel } from "../../features/terminal/terminal-panel"
import { Brand } from "../ui/brand"
import { Button, IconButton } from "../ui/button"
import { Modal } from "../ui/modal"
import { SplitPane } from "../ui/split-pane"
import { EnvironmentView } from "./environment-view"
import { ProfileMenu } from "./profile-menu"
import "./workspace.css"

const compactQuery = "(max-width: 900px)"
function subscribeCompact(listener: () => void) {
  const query = window.matchMedia(compactQuery)
  query.addEventListener("change", listener)
  return () => query.removeEventListener("change", listener)
}
const readCompact = () => window.matchMedia(compactQuery).matches
const hasRecoverableRuns = async (signal: AbortSignal) =>
  (await listRecoverable(signal)).length > 0

export function WorkspacePage() {
  const environment = useEnvironment(hasRecoverableRuns)
  const available =
    environment.state.environment?.status === "running" &&
    environment.state.pending !== "stop"
  const connection = useConnection(available)
  const [claudeSetup, setClaudeSetup] = useState(false)
  const claude = useClaudeConnection(available, claudeSetup)
  const codexConnected = connection.state.connection?.status === "connected"
  const claudeConnected = claude.data?.status === "connected"
  const connected = codexConnected || claudeConnected
  const [wasConnected, setWasConnected] = useState(false)
  if (connected && !wasConnected) setWasConnected(true)
  if (
    available &&
    connection.state.connection?.status === "disconnected" &&
    claude.data?.status === "disconnected" &&
    wasConnected
  )
    setWasConnected(false)
  const compact = useSyncExternalStore(subscribeCompact, readCompact)
  const [desktopTerminal, setDesktopTerminal] = useState(true)
  const [mobileTerminal, setMobileTerminal] = useState(false)
  const [sidebarVisible, setSidebarVisible] = useState(true)
  const [terminalExpanded, setTerminalExpanded] = useState(false)
  const [drawerOpen, setDrawerOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [stopOpen, setStopOpen] = useState(false)
  useEffect(() => {
    if (claudeSetup && claudeConnected) setClaudeSetup(false)
  }, [claudeConnected, claudeSetup])
  const entryContent = useRef<HTMLDivElement>(null)
  const claudeHeading = useRef<HTMLHeadingElement>(null)
  const previousClaudeSetup = useRef(false)
  useEffect(() => {
    if (claudeSetup && !connected) claudeHeading.current?.focus()
    else if (previousClaudeSetup.current && !connected)
      entryContent.current?.focus()
    previousClaudeSetup.current = claudeSetup
  }, [claudeSetup, connected])
  const previousAvailable = useRef(available)
  useEffect(() => {
    if (previousAvailable.current && !available) entryContent.current?.focus()
    previousAvailable.current = available
  }, [available])
  const navigationToggle = useRef<HTMLButtonElement>(null)
  const terminalToggle = useRef<HTMLButtonElement>(null)
  const terminalVisible = compact ? mobileTerminal : desktopTerminal
  const workspaceReady = available && connected
  // Keep established content while an observation is unknown or being retried.
  // Definite environment/provider transitions still use their entry screens.
  const resuming =
    wasConnected &&
    ((!environment.state.environment &&
      (!environment.state.pending || environment.state.pending === "read")) ||
      (available &&
        !connection.state.connection &&
        (!connection.state.pending || connection.state.pending === "read")))
  const resumeIssue = resuming
    ? (environment.state.message ?? connection.state.message)
    : null
  const showWorkspace = workspaceReady || resuming
  const expanded = terminalExpanded && showWorkspace && terminalVisible
  useEffect(() => {
    if (!showWorkspace || !available) setTerminalExpanded(false)
  }, [showWorkspace, available])
  const canStop =
    canStopEnvironment(environment.state.environment) &&
    (!environment.state.pending || environment.state.pending === "read") &&
    !environment.state.paused
  function changeTerminal(visible: boolean) {
    if (!visible) setTerminalExpanded(false)
    if (compact) setMobileTerminal(visible)
    else setDesktopTerminal(visible)
    if (!visible) terminalToggle.current?.focus()
  }
  function openSettings() {
    setDrawerOpen(false)
    setSettingsOpen(true)
  }
  function openStop() {
    setDrawerOpen(false)
    setStopOpen(true)
  }
  function connectClaude() {
    setSettingsOpen(false)
    setClaudeSetup(true)
    if (claude.data?.status !== "connecting") void claude.begin()
  }
  async function closeClaudeSetup() {
    if (claude.pending) return
    if (claude.data?.challenge && !(await claude.cancel())) return
    setClaudeSetup(false)
  }
  const profile = (
    <ProfileMenu
      canStop={canStop}
      onSettings={openSettings}
      onStop={openStop}
    />
  )
  function layout(parts?: {
    navigation: ReactNode
    conversation: ReactNode
    title: string
  }) {
    return (
      <div
        className={showWorkspace ? "workspace" : "workspace workspace-entry"}
        data-terminal-expanded={expanded ? "" : undefined}
      >
        {showWorkspace && !compact ? (
          <div
            className="workspace-sidebar-track t-resize"
            data-collapsed={!sidebarVisible || expanded ? "" : undefined}
            inert={!sidebarVisible || expanded}
            aria-hidden={!sidebarVisible || expanded}
          >
            <aside
              id="workspace-sidebar"
              className="workspace-sidebar"
              aria-label="Workspace navigation"
            >
              <Brand />
              {parts?.navigation}
              {profile}
            </aside>
          </div>
        ) : !showWorkspace ? (
          <>
            <div className="workspace-entry-brand">
              <Brand />
            </div>
            <div className="workspace-entry-profile">{profile}</div>
          </>
        ) : null}
        {showWorkspace ? (
          <div className="workspace-body">
            <div
              className="workspace-header-track t-resize"
              inert={expanded}
              aria-hidden={expanded}
            >
              <header className="workspace-header">
                <IconButton
                  ref={navigationToggle}
                  label={
                    compact
                      ? "Open chats"
                      : sidebarVisible
                        ? "Hide chats"
                        : "Show chats"
                  }
                  aria-expanded={compact ? drawerOpen : sidebarVisible}
                  aria-controls={
                    compact ? "workspace-drawer-content" : "workspace-sidebar"
                  }
                  onClick={() =>
                    compact
                      ? setDrawerOpen(true)
                      : setSidebarVisible((visible) => !visible)
                  }
                >
                  {compact ? (
                    <PanelLeft aria-hidden="true" />
                  ) : sidebarVisible ? (
                    <PanelLeftClose aria-hidden="true" />
                  ) : (
                    <PanelLeftOpen aria-hidden="true" />
                  )}
                </IconButton>
                <h1>{parts?.title ?? "New chat"}</h1>
                <IconButton
                  ref={terminalToggle}
                  label={terminalVisible ? "Hide terminal" : "Show terminal"}
                  aria-expanded={terminalVisible}
                  aria-controls="workspace-terminal"
                  onClick={() => changeTerminal(!terminalVisible)}
                >
                  <Terminal aria-hidden="true" />
                </IconButton>
              </header>
            </div>
            {resumeIssue && (
              <div className="workspace-notice" role="status">
                <p>{resumeIssue}</p>
                {available && connection.state.message && (
                  <Button
                    variant="ghost"
                    disabled={
                      connection.state.pending !== null ||
                      connection.state.paused
                    }
                    onClick={() => void connection.controller.refresh()}
                  >
                    Check again
                  </Button>
                )}
              </div>
            )}
            <SplitPane
              compact={compact}
              primary={parts?.conversation}
              secondary={
                <TerminalPanel
                  available={available}
                  visible={terminalVisible}
                  expanded={expanded}
                  onExpandedChange={setTerminalExpanded}
                />
              }
              secondaryVisible={terminalVisible}
              secondaryExpanded={expanded}
              onSecondaryVisibleChange={changeTerminal}
              secondaryId="workspace-terminal"
            />
          </div>
        ) : (
          <div
            ref={entryContent}
            tabIndex={-1}
            className="workspace-entry-content"
          >
            {available ? (
              claudeSetup ? (
                <ClaudeLogin
                  connection={claude}
                  headingRef={claudeHeading}
                  onClose={() => setClaudeSetup(false)}
                />
              ) : (
                <ProviderConnection
                  {...connection}
                  onClaude={connectClaude}
                  claudeAvailable={available}
                />
              )
            ) : (
              <EnvironmentView {...environment} />
            )}
          </div>
        )}
        <Modal
          open={compact && drawerOpen && showWorkspace}
          onClose={() => setDrawerOpen(false)}
          title="Chats"
          className="workspace-drawer"
        >
          <div className="workspace-drawer-top">
            <Brand />
            <IconButton
              label="Close chats"
              data-autofocus
              onClick={() => setDrawerOpen(false)}
            >
              <X aria-hidden="true" />
            </IconButton>
          </div>
          <div
            id="workspace-drawer-content"
            className="workspace-drawer-navigation"
            onClick={(event) => {
              if (
                (event.target as Element).closest(
                  ".conversation-row, .conversation-new"
                )
              ) {
                setMobileTerminal(false)
                setDrawerOpen(false)
              }
            }}
          >
            {parts?.navigation}
          </div>
          {profile}
        </Modal>
        <Modal
          returnFocusRef={compact ? navigationToggle : undefined}
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          title="Settings"
        >
          <div className="workspace-settings">
            <p>Dark · Space Mono</p>
            {available ? (
              <>
                {codexConnected ? (
                  <ConnectedProvider {...connection} />
                ) : (
                  <ProviderConnection {...connection} inSettings />
                )}
                <ClaudeSettings
                  connection={claude}
                  available={available}
                  onConnect={connectClaude}
                />
              </>
            ) : (
              <p>
                Provider settings are available when your workspace is running.
              </p>
            )}
          </div>
          <div className="ui-modal-actions">
            <Button data-autofocus onClick={() => setSettingsOpen(false)}>
              Done
            </Button>
          </div>
        </Modal>
        <Modal
          open={claudeSetup && connected}
          onClose={() => void closeClaudeSetup()}
          title="Connect Claude Code"
        >
          <ClaudeLogin
            connection={claude}
            onClose={() => setClaudeSetup(false)}
          />
        </Modal>
        <Modal
          returnFocusRef={compact ? navigationToggle : undefined}
          open={stopOpen}
          onClose={() => setStopOpen(false)}
          title="Stop your environment?"
          descriptionId="stop-environment-description"
        >
          <p id="stop-environment-description">
            Running tasks and terminal processes will stop. Saved files and
            completed chat messages are kept. You can return to your workspace
            later.
          </p>
          <div className="ui-modal-actions">
            <Button data-autofocus onClick={() => setStopOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              disabled={!canStop}
              onClick={() => {
                setStopOpen(false)
                void environment.controller.stop()
              }}
            >
              Stop environment
            </Button>
          </div>
        </Modal>
      </div>
    )
  }
  return connected || wasConnected ? (
    <Chat
      available={workspaceReady}
      connectedProviders={[
        ...(codexConnected ? [{ id: "codex", name: "Codex" }] : []),
        ...(claudeConnected ? [{ id: "claude", name: "Claude" }] : []),
      ]}
      resuming={resuming}
      renderLayout={layout}
    />
  ) : (
    layout()
  )
}
