import * as m from "@workspace/i18n/messages"
import { useNavigate, useSearch } from "@tanstack/react-router"
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
  House,
  GitBranch,
  MessageSquare,
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
import { FeatureHome } from "../../features/feature-workflows/feature-home"
import { FeatureWorkspace } from "../../features/feature-workflows/feature-workspace"
import { useFeatureWorkspace } from "../../features/feature-workflows/use-feature-workspace"
import * as featureApi from "../../features/feature-workflows/api"
import { LoadingState } from "../ui/loading-state"
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
  const search = useSearch({ from: "/authenticated/app" })
  const navigate = useNavigate({ from: "/app" })
  const mode = search.view ?? "overview"
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
  const [desktopTerminal, setDesktopTerminal] = useState(mode === "chat")
  const [mobileTerminal, setMobileTerminal] = useState(false)
  const [sidebarVisible, setSidebarVisible] = useState(true)
  const [terminalExpanded, setTerminalExpanded] = useState(false)
  const [drawerOpen, setDrawerOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [stopOpen, setStopOpen] = useState(false)
  const [featureProject, setFeatureProject] = useState("")
  const [detailHeader, setDetailHeader] = useState<HTMLElement | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [focusFeature, setFocusFeature] = useState<string>()
  const [createProject, setCreateProject] = useState("")
  const [createProvider, setCreateProvider] =
    useState<featureApi.FeatureProvider>("codex")
  const featureState = useFeatureWorkspace({
    available: available && connected,
    featureId: mode === "features" ? search.feature : undefined,
    projectId: mode === "features" ? featureProject || undefined : undefined,
  })
  const selectedFeature =
    mode === "features" && search.feature ? featureState.feature : undefined
  const providers: { id: featureApi.FeatureProvider; name: string }[] = [
    ...(codexConnected ? [{ id: "codex" as const, name: "Codex" }] : []),
    ...(claudeConnected
      ? [{ id: "claude" as const, name: "Claude Code" }]
      : []),
  ]
  function browse(view: "overview" | "chat" | "features", feature?: string) {
    setDrawerOpen(false)
    setMobileTerminal(false)
    if (view !== "chat") setDesktopTerminal(false)
    void navigate({ search: { view, ...(feature ? { feature } : {}) } })
  }
  async function createFeature(
    project: string,
    provider: featureApi.FeatureProvider
  ) {
    const created = await featureState.create(project, provider)
    if (created) {
      setCreateOpen(false)
      setFocusFeature(created.feature.id)
      projects.setSelectedProjectId(project)
      browse("features", created.feature.id)
    }
  }
  function openFeatureCreation() {
    const ready = projects.projects.filter(
      (project) => project.state === "ready"
    )
    const project =
      ready.find((item) => item.id === projects.selectedProjectId) ??
      (ready.length === 1 ? ready[0] : undefined)
    const provider = providers[0]?.id ?? "codex"
    setCreateProject(project?.id ?? "")
    setCreateProvider(provider)
    if (project && providers.length === 1)
      void createFeature(project.id, provider)
    else setCreateOpen(true)
  }
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
  const modeNavigation = (
    <nav className="workspace-modes" aria-label={m.workspace_modes()}>
      <Button
        variant="ghost"
        aria-current={mode === "overview" ? "page" : undefined}
        onClick={() => browse("overview")}
      >
        <House size={16} aria-hidden="true" />
        {m.workspace_overview()}
      </Button>
      <Button
        variant="ghost"
        data-mode="chat"
        aria-current={mode === "chat" ? "page" : undefined}
        onClick={() => browse("chat")}
      >
        <MessageSquare size={16} aria-hidden="true" />
        {m.chat()}
      </Button>
      <Button
        variant="ghost"
        data-mode="feature"
        aria-current={mode === "features" ? "page" : undefined}
        onClick={() => browse("features")}
      >
        <GitBranch size={16} aria-hidden="true" />
        {m.features()}
      </Button>
    </nav>
  )
  function layout(parts?: {
    navigation: ReactNode
    conversation: ReactNode
    title: string
    conversationId?: string
    projectName?: string
    projectWarnings?: string[]
  }) {
    const title =
      mode === "chat"
        ? (parts?.title ?? m.new_chat())
        : mode === "features"
          ? m.features()
          : m.workspace_overview()
    const content =
      mode === "chat" ? (
        parts?.conversation
      ) : mode === "features" && search.feature ? (
        selectedFeature ? (
          <FeatureWorkspace
            key={selectedFeature.feature.id}
            state={featureState}
            discussion={parts?.conversation}
            headerHost={detailHeader}
            projectName={parts?.projectName}
            available={available && connected}
            onBack={() => browse("features")}
          />
        ) : (
          <div className="feature-home">
            {featureState.featureError ? (
              <div className="chat-notice" role="status">
                <p>{m.feature_load_error()}</p>
                <Button
                  disabled={!available}
                  onClick={() => void featureState.refresh()}
                >
                  {m.retry()}
                </Button>
                <Button variant="ghost" onClick={() => browse("features")}>
                  {m.feature_back()}
                </Button>
              </div>
            ) : (
              <LoadingState title={m.loading()} />
            )}
          </div>
        )
      ) : (
        <FeatureHome
          state={featureState}
          overview={mode === "overview"}
          available={available && connected}
          projects={projects.projects}
          projectFilter={featureProject}
          onProjectFilter={setFeatureProject}
          onCreate={openFeatureCreation}
          onFeature={(id) => browse("features", id)}
          onChat={() => browse("chat")}
          onFeatures={() => browse("features")}
        />
      )
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
            {modeNavigation}
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
                {mode === "features" && search.feature ? (
                  <div
                    className="workspace-feature-header-host"
                    ref={setDetailHeader}
                  >
                    {!selectedFeature && <h1>{title}</h1>}
                  </div>
                ) : (
                  <h1>{title}</h1>
                )}
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
              primary={content}
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
            {modeNavigation}
            {compact ? parts?.navigation : null}
          </div>
          {profile}
        </Modal>
        <Modal
          open={createOpen}
          onClose={() => {
            if (!featureState.busy) setCreateOpen(false)
          }}
          title={m.new_feature()}
        >
          <p>{m.feature_create_context()}</p>
          {projects.projects.some((project) => project.state === "ready") ? (
            <div className="feature-create-fields">
              <label>
                {m.feature_project()}
                <select
                  value={createProject}
                  onChange={(event) => setCreateProject(event.target.value)}
                  disabled={featureState.busy}
                >
                  <option value="">{m.feature_project()}</option>
                  {projects.projects
                    .filter((project) => project.state === "ready")
                    .map((project) => (
                      <option key={project.id} value={project.id}>
                        {project.fullName}
                      </option>
                    ))}
                </select>
              </label>
              <label>
                {m.feature_provider()}
                <select
                  value={createProvider}
                  onChange={(event) =>
                    setCreateProvider(
                      event.target.value as featureApi.FeatureProvider
                    )
                  }
                  disabled={featureState.busy}
                >
                  {providers.map((provider) => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          ) : (
            <p>{m.feature_no_project()}</p>
          )}
          {featureState.message && (
            <p className="chat-notice" role="status">
              {featureState.message}
            </p>
          )}
          <div className="ui-modal-actions">
            <Button
              onClick={() => setCreateOpen(false)}
              disabled={featureState.busy}
            >
              {m.cancel()}
            </Button>
            <Button
              variant="primary"
              disabled={
                !createProject ||
                !providers.some((provider) => provider.id === createProvider) ||
                featureState.busy ||
                !available
              }
              onClick={() => void createFeature(createProject, createProvider)}
            >
              {featureState.busy ? m.loading() : m.feature_continue()}
            </Button>
          </div>
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
      onOpenChat={() => browse("chat")}
      navigationActive={mode === "chat"}
      workflow={
        selectedFeature
          ? {
              conversation: {
                id: selectedFeature.feature.conversationId,
                projectId: selectedFeature.feature.projectId,
                createdAt: selectedFeature.feature.createdAt,
              },
              modelLabel: (() => {
                const attempt = selectedFeature.attempts
                  .slice()
                  .reverse()
                  .find(
                    (item) =>
                      item.role ===
                      (selectedFeature.feature.stage === "architecture"
                        ? "architecture"
                        : "coordinator")
                  )
                return (
                  attempt?.observedModel ??
                  attempt?.assignedModel ??
                  m.feature_profile()
                )
              })(),
              start: (operationId, input, csrf, signal) =>
                featureApi.start(
                  selectedFeature.feature.id,
                  { operationId, input },
                  csrf,
                  signal
                ),
              onRefresh: featureState.refresh,
              version: selectedFeature.attempts
                .map((attempt) => `${attempt.id}:${attempt.status}`)
                .join(","),
              focusOnReady: focusFeature === selectedFeature.feature.id,
              onFocused: () => setFocusFeature(undefined),
            }
          : undefined
      }
    />
  ) : (
    layout()
  )
}
