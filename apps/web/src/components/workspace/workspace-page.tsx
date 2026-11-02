import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react"
import { setLanguage, type Language } from "../../lib/i18n"
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
import { hasRecoverableWork } from "../../features/projects/api"
import { useProjects } from "../../features/projects/use-projects"
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
import { WorkspaceSidebar } from "./workspace-sidebar"
import "./workspace.css"

const compactQuery = "(max-width: 900px)"
function subscribeCompact(listener: () => void) {
  const query = window.matchMedia(compactQuery)
  query.addEventListener("change", listener)
  return () => query.removeEventListener("change", listener)
}
const readCompact = () => window.matchMedia(compactQuery).matches

export function WorkspacePage() {
  const locale = useLocale()
  const environment = useEnvironment(hasRecoverableWork)
  const available =
    environment.state.environment?.status === "running" &&
    environment.state.pending !== "stop"
  const projects = useProjects(available)
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
    conversationId?: string
    projectName?: string
    projectWarnings?: string[]
  }) {
    return (
      <div
        className={showWorkspace ? "workspace" : "workspace workspace-entry"}
        data-terminal-expanded={expanded ? "" : undefined}
      >
        {showWorkspace ? (
          <WorkspaceSidebar
            visible={sidebarVisible && !expanded}
            compact={compact}
            terminalVisible={terminalVisible}
            onCollapse={() => {
              setSidebarVisible(false)
              navigationToggle.current?.focus({ preventScroll: true })
            }}
          >
            <Brand />
            {parts?.navigation}
            {profile}
          </WorkspaceSidebar>
        ) : (
          <>
            <div className="workspace-entry-brand">
              <Brand />
            </div>
            <div className="workspace-entry-profile">{profile}</div>
          </>
        )}
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
                      ? m.open_chats()
                      : sidebarVisible
                        ? m.hide_chats()
                        : m.show_chats()
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
                <h1>{parts?.title ?? m.new_chat()}</h1>
                <IconButton
                  ref={terminalToggle}
                  label={
                    terminalVisible ? m.hide_terminal() : m.show_terminal()
                  }
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
                <p>{translateMessage(resumeIssue)}</p>
                {available && connection.state.message && (
                  <Button
                    variant="ghost"
                    disabled={
                      connection.state.pending !== null ||
                      connection.state.paused
                    }
                    onClick={() => void connection.controller.refresh()}
                  >
                    {m.check_again()}
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
                  conversationId={parts?.conversationId}
                  projectName={parts?.projectName}
                  projectWarnings={parts?.projectWarnings}
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
          title={m.chats()}
          className="workspace-drawer"
        >
          <div className="workspace-drawer-top">
            <Brand />
            <IconButton
              label={m.close_chats()}
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
                  ".conversation-row, .conversation-new, .project-create-chat"
                )
              ) {
                setMobileTerminal(false)
                setDrawerOpen(false)
              }
            }}
          >
            {compact ? parts?.navigation : null}
          </div>
          {profile}
        </Modal>
        <Modal
          returnFocusRef={compact ? navigationToggle : undefined}
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          title={m.settings()}
        >
          <div className="workspace-settings">
            <p>Space Mono</p>
            <label>
              {m.language()}
              <select
                value={locale}
                onChange={(event) =>
                  setLanguage(event.target.value as Language)
                }
              >
                <option value="en">English</option>
                <option value="fr">Français</option>
              </select>
            </label>
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
                {m.provider_settings_are_available_when_your_workspace_is_running()}
              </p>
            )}
          </div>
          <div className="ui-modal-actions">
            <Button data-autofocus onClick={() => setSettingsOpen(false)}>
              {m.done()}
            </Button>
          </div>
        </Modal>
        <Modal
          open={claudeSetup && connected}
          onClose={() => void closeClaudeSetup()}
          title={m.connect_claude_code()}
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
          title={m.stop_your_environment()}
          descriptionId="stop-environment-description"
        >
          <p id="stop-environment-description">
            {m.running_tasks_and_terminal_processes_will_stop_saved_files_and_co()}
          </p>
          <div className="ui-modal-actions">
            <Button data-autofocus onClick={() => setStopOpen(false)}>
              {m.cancel()}
            </Button>
            <Button
              variant="danger"
              disabled={!canStop}
              onClick={() => {
                setStopOpen(false)
                void environment.controller.stop()
              }}
            >
              {m.stop_environment()}
            </Button>
          </div>
        </Modal>
      </div>
    )
  }
  return connected || wasConnected ? (
    <Chat
      available={workspaceReady}
      projectWorkspace={projects}
      connectedProviders={[
        ...(codexConnected ? [{ id: "codex", name: "Codex" }] : []),
        ...(claudeConnected ? [{ id: "claude", name: "Claude" }] : []),
      ]}
      resuming={resuming}
      navigationVisible={!compact || drawerOpen}
      renderLayout={layout}
    />
  ) : (
    layout()
  )
}
