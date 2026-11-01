import { useEffect, useLayoutEffect, useRef, useState } from "react"
import {
  ChevronDown,
  ChevronRight,
  FolderGit2,
  Plus,
  SquarePen,
  Trash2,
} from "lucide-react"

import { Button, IconButton } from "../../components/ui/button"
import { Modal } from "../../components/ui/modal"
import { LoadingState } from "../../components/ui/loading-state"
import type { Conversation } from "../conversations/api"
import { conversationLabel } from "../conversations/conversation-navigation"
import type { ProjectWorkspace } from "./use-projects"
import "./projects.css"

export function ProjectNavigation({
  workspace,
  available,
  busy,
  onNewChat,
  conversations,
  selectedConversation,
  onSelectConversation,
  onProjectRemoved,
}: {
  workspace: ProjectWorkspace
  available: boolean
  busy: boolean
  onNewChat: (projectId: string) => void
  conversations: Conversation[]
  selectedConversation: string | undefined
  onSelectConversation: (id: string) => void
  onProjectRemoved: (projectId: string) => void
}) {
  const [search, setSearch] = useState("")
  const [deleteTarget, setDeleteTarget] = useState<string>()
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () =>
      new Set(workspace.selectedProjectId ? [workspace.selectedProjectId] : [])
  )
  useEffect(() => {
    const id = workspace.selectedProjectId
    if (!id) return
    setExpandedProjects((current) => {
      if (current.has(id)) return current
      return new Set([...current, id])
    })
  }, [workspace.selectedProjectId])
  const addButton = useRef<HTMLButtonElement>(null)
  const deleteFocus = useRef<HTMLElement>(null)
  const closeButton = useRef<HTMLButtonElement>(null)
  useLayoutEffect(() => {
    // A focused transient control may be removed after an asynchronous reply.
    // Do not move focus when the user is already on another live control.
    if (workspace.pickerOpen && document.activeElement === document.body)
      closeButton.current?.focus()
  }, [
    workspace.pickerOpen,
    workspace.connectionLoading,
    workspace.connection?.status,
    workspace.repositoryLoading,
    workspace.repositoryMessage,
    workspace.busy,
    workspace.importProgress,
  ])
  const matching = workspace.repositories.filter((repository) =>
    repository.fullName
      .toLocaleLowerCase()
      .includes(search.trim().toLocaleLowerCase())
  )
  const progress = workspace.importProgress
  const trackedImport = workspace.projects.find(
    (project) => project.id === progress?.projectId
  )
  const importInterrupted =
    (Boolean(progress) && !available) ||
    progress?.checkFailed ||
    trackedImport?.state === "uncertain" ||
    trackedImport?.state === "failed"
  const target = workspace.projects.find(
    (project) => project.id === deleteTarget
  )
  return (
    <section className="project-navigation" aria-label="Projects">
      <div className="project-navigation-heading">
        <p className="conversation-list-label">Projects</p>
        <IconButton
          ref={addButton}
          label="Add project"
          disabled={!available || workspace.busy}
          onClick={() => {
            setSearch("")
            workspace.openPicker()
          }}
        >
          <Plus size={16} aria-hidden="true" />
        </IconButton>
      </div>
      {workspace.loading && !workspace.projects.length && (
        <p className="project-hint" role="status">
          Loading projects…
        </p>
      )}
      {!workspace.loading && !workspace.projects.length && (
        <p className="project-hint">No projects yet.</p>
      )}
      <nav className="project-list" aria-label="Your projects">
        {workspace.projects.map((project) => (
          <div className="project-item" key={project.id}>
            <div className="project-row-group">
              <Button
                variant="ghost"
                className="project-row"
                aria-expanded={expandedProjects.has(project.id)}
                onClick={() => {
                  const expanded = expandedProjects.has(project.id)
                  setExpandedProjects((current) => {
                    const next = new Set(current)
                    if (expanded) next.delete(project.id)
                    else next.add(project.id)
                    return next
                  })
                  if (!expanded) workspace.setSelectedProjectId(project.id)
                }}
              >
                {expandedProjects.has(project.id) ? (
                  <ChevronDown aria-hidden="true" size={13} />
                ) : (
                  <ChevronRight aria-hidden="true" size={13} />
                )}
                <FolderGit2 aria-hidden="true" size={15} />
                <span title={project.fullName}>
                  {project.fullName.split("/").at(-1)}
                </span>
              </Button>
              <IconButton
                className="project-create-chat"
                label={`New chat in ${project.fullName}`}
                disabled={!available || busy || project.state !== "ready"}
                onClick={() => {
                  workspace.setSelectedProjectId(project.id)
                  setExpandedProjects(
                    (current) => new Set([...current, project.id])
                  )
                  onNewChat(project.id)
                }}
              >
                <SquarePen size={15} aria-hidden="true" />
              </IconButton>
              <IconButton
                className="project-delete"
                label={`Delete ${project.fullName}`}
                disabled={
                  !available ||
                  workspace.busy ||
                  busy ||
                  project.state === "importing" ||
                  project.state === "uncertain"
                }
                onClick={(event) => {
                  deleteFocus.current = event.currentTarget
                  setDeleteTarget(project.id)
                }}
              >
                <Trash2 size={15} aria-hidden="true" />
              </IconButton>
            </div>
            {expandedProjects.has(project.id) && (
              <div
                className="project-chats conversation-list"
                aria-label={`Chats in ${project.fullName}`}
              >
                {conversations
                  .filter(
                    (conversation) => conversation.projectId === project.id
                  )
                  .map((conversation) => (
                    <Button
                      key={conversation.id}
                      variant="ghost"
                      className="conversation-row"
                      aria-current={
                        conversation.id === selectedConversation
                          ? "page"
                          : undefined
                      }
                      title={conversationLabel(conversation)}
                      disabled={busy || !available}
                      onClick={() => onSelectConversation(conversation.id)}
                    >
                      <span>{conversationLabel(conversation)}</span>
                    </Button>
                  ))}
              </div>
            )}
            {project.state !== "ready" && (
              <p className="project-state" role="status">
                {project.state === "importing"
                  ? "Importing…"
                  : project.state === "uncertain"
                    ? "Import interrupted. Stop the workspace and recover it before retrying."
                    : project.state === "deleting"
                      ? "Deletion interrupted. Delete again to finish."
                      : "Import needs attention."}
              </p>
            )}
            {project.id === workspace.selectedProjectId &&
              project.state === "ready" &&
              workspace.selectedVerification !== "matched" &&
              workspace.selectedVerification !== "checking" && (
                <p className="project-state" role="status">
                  {workspace.selectedVerification === "changed"
                    ? "The original GitHub address changed or is inaccessible. Check the Git remote before pushing."
                    : "GitHub address not verified; check your Git remote before pushing."}
                </p>
              )}
            {project.renamedSinceClone && (
              <p className="project-state" role="status">
                Repository renamed since import. Verify the Git remote before
                pushing.
              </p>
            )}
            {project.state === "failed" && (
              <Button
                variant="ghost"
                className="project-retry"
                disabled={!available || workspace.busy}
                onClick={() => workspace.retry(project.id)}
              >
                Retry import
              </Button>
            )}
          </div>
        ))}
      </nav>
      {workspace.nextCursor && (
        <Button
          variant="ghost"
          className="project-more"
          disabled={!available || workspace.busy || workspace.loading}
          onClick={workspace.loadMoreProjects}
        >
          Older projects
        </Button>
      )}
      {workspace.message && (
        <div className="project-message" role="status">
          <p>{workspace.message}</p>
          {workspace.needsCheck && (
            <Button
              variant="ghost"
              disabled={!available || workspace.busy}
              onClick={workspace.refresh}
            >
              Check projects
            </Button>
          )}
        </div>
      )}
      <Modal
        open={Boolean(target)}
        onClose={() => setDeleteTarget(undefined)}
        title="Delete project?"
        returnFocusRef={deleteFocus}
        className="project-delete-dialog"
      >
        <p>
          This permanently deletes <strong>{target?.fullName}</strong>, its
          local folder in Documents, and all its chats. Unpushed changes are
          lost. Stop terminal commands before deleting. The GitHub repository
          stays intact.
        </p>
        <div className="project-delete-actions">
          <Button data-autofocus onClick={() => setDeleteTarget(undefined)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={!target || workspace.busy}
            onClick={() => {
              if (!target) return
              const id = target.id
              setDeleteTarget(undefined)
              void workspace.removeProject(id).then((removed) => {
                if (removed) onProjectRemoved(id)
              })
            }}
          >
            Delete project
          </Button>
        </div>
      </Modal>
      <Modal
        open={workspace.pickerOpen}
        onClose={workspace.closePicker}
        title={progress ? "Import project" : "Add a project"}
        returnFocusRef={addButton}
        className="project-picker"
      >
        {progress ? (
          <div className="project-import-progress">
            {importInterrupted ? (
              <div role="status" aria-live="polite">
                <p className="project-import-title">
                  Import could not be confirmed
                </p>
                <p>
                  Check the project list before trying again. If the import was
                  interrupted, stop the workspace and recover it before
                  retrying.
                </p>
              </div>
            ) : (
              <>
                <LoadingState title="Importing project…" />
                <p>
                  Cloning <strong>{progress.fullName}</strong> into Documents.
                </p>
              </>
            )}
          </div>
        ) : workspace.connection?.status === "connected" ? (
          <>
            <p className="project-picker-caption">
              Choose a GitHub repository to clone into your workspace.
            </p>
            <label className="project-search">
              Search repositories
              <input
                data-autofocus
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                autoComplete="off"
              />
            </label>
            <div
              className="project-repositories"
              aria-label="Importable repositories"
              aria-busy={workspace.repositoryLoading || workspace.busy}
            >
              {matching.map((repository) => (
                <Button
                  key={repository.id}
                  variant="ghost"
                  className="project-repository"
                  disabled={!available || workspace.busy}
                  onClick={() => workspace.importRepository(repository)}
                >
                  <span>{repository.fullName}</span>
                  <span className="project-repository-meta">
                    <small>{repository.private ? "Private" : "Public"}</small>
                    <ChevronRight size={16} aria-hidden="true" />
                  </span>
                </Button>
              ))}
              {!matching.length &&
                !workspace.repositoryLoading &&
                !workspace.repositoryMessage && (
                  <p className="project-hint">
                    {search
                      ? "No matches in the loaded repositories."
                      : "No repositories to import."}
                  </p>
                )}
            </div>
            <p className="project-hint">
              Search includes loaded repositories.{" "}
              {workspace.nextPage
                ? "Load more to search additional repositories."
                : ""}
            </p>
            {workspace.repositoryLoading && (
              <p className="project-hint" role="status">
                Loading repositories…
              </p>
            )}
            {workspace.repositoryMessage && (
              <div className="project-message" role="status">
                <p>{workspace.repositoryMessage}</p>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.refreshRepositories}
                >
                  Try again
                </Button>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.connect}
                >
                  Reconnect GitHub
                </Button>
              </div>
            )}
            {workspace.nextPage && (
              <Button
                variant="ghost"
                disabled={workspace.repositoryLoading || workspace.busy}
                onClick={workspace.loadMoreRepositories}
              >
                Load more
              </Button>
            )}
            <details className="project-account">
              <summary>GitHub access</summary>
              <p>
                Connected as {workspace.connection.login}. Git commits include
                your GitHub email. Disconnecting here keeps your projects and
                may leave the SSH key active on GitHub.
              </p>
              <Button
                variant="ghost"
                disabled={workspace.busy}
                onClick={workspace.disconnect}
              >
                Disconnect GitHub access
              </Button>
              <a
                href="https://github.com/settings/keys"
                target="_blank"
                rel="noreferrer"
              >
                Review SSH keys on GitHub
              </a>
            </details>
          </>
        ) : workspace.connectionLoading && !workspace.connection ? (
          <p className="project-picker-caption" role="status">
            Checking GitHub access…
          </p>
        ) : workspace.connection?.status === "disconnected" ? (
          <div className="project-connect">
            <p>
              Connect the same GitHub account to list and clone your
              repositories.
            </p>
            <p>
              The GitHub authorization grants Factory broad read and write
              access to your private repositories and some organization
              resources. It also allows Factory to add an SSH key and read your
              GitHub email. The SSH key lets the workspace use your GitHub
              permissions. Your commit email is visible to anyone who can read
              those commits.
            </p>
            <Button
              data-autofocus
              variant="primary"
              disabled={!available || workspace.busy}
              onClick={workspace.connect}
            >
              {workspace.busy ? "Connecting…" : "Connect GitHub"}
            </Button>
          </div>
        ) : (
          <div className="project-connect">
            <p role="status">GitHub connection status is unavailable.</p>
            <Button
              data-autofocus
              disabled={workspace.busy}
              onClick={workspace.refresh}
            >
              Try again
            </Button>
          </div>
        )}
        <div className="ui-modal-actions">
          <Button ref={closeButton} onClick={workspace.closePicker}>
            {progress && !importInterrupted
              ? "Continue in background"
              : "Close"}
          </Button>
        </div>
      </Modal>
    </section>
  )
}
