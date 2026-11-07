import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { useQuery, useQueryClient } from "@tanstack/react-query"
import { ArrowDown, SquarePen } from "lucide-react"

import { Button } from "../../components/ui/button"
import { LoadingState } from "../../components/ui/loading-state"
import { ProjectNavigation } from "../projects/project-navigation"
import type { ProjectWorkspace } from "../projects/use-projects"
import { projectWarnings } from "../projects/state"
import { useProjectVerification } from "../projects/use-project-verification"
import * as projectApi from "../projects/api"
import { ChatComposer } from "./chat-composer"
import {
  ConversationNavigation,
  conversationLabel,
} from "./conversation-navigation"

import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import * as api from "./api"
import { modelQueryKey, modelQueryOptions } from "./model-query"
import { MessageMarkdown } from "./message-markdown"
import {
  latestReader,
  runOutcome,
  questionReceipt,
  mergeOlderConversations,
  selectedEffort,
} from "./state"
import "./conversations.css"

type PendingSend = {
  operationId: string
  input: string
  model: string
  effort: string
  conversation: string
}

export type ChatLayout = {
  navigation: ReactNode
  conversation: ReactNode
  title: string
  conversationId?: string
  projectName?: string
  projectWarnings?: string[]
}

export type WorkflowConversation = {
  conversation: api.Conversation
  modelLabel: string
  start: (
    operationId: string,
    input: string,
    csrf: string,
    signal: AbortSignal
  ) => Promise<unknown>
  onRefresh: () => void
  version: string
  focusOnReady?: boolean
  onFocused?: () => void
}

export function Chat({
  available,
  projectWorkspace,
  connectedProviders,
  resuming = false,
  navigationVisible = true,
  renderLayout,
  workflow,
  onOpenChat,
  navigationActive = true,
}: {
  available: boolean
  projectWorkspace: ProjectWorkspace
  connectedProviders: { id: string; name: string }[]
  resuming?: boolean
  navigationVisible?: boolean
  renderLayout: (parts: ChatLayout) => ReactNode
  workflow?: WorkflowConversation
  onOpenChat?: () => void
  navigationActive?: boolean
}) {
  const locale = useLocale()
  const { state: session, controller: sessionController } = useSession()
  const queryClient = useQueryClient()
  const closeProjectPicker = projectWorkspace.setPickerOpen
  useEffect(() => {
    if (!navigationVisible) closeProjectPicker(false)
  }, [navigationVisible, closeProjectPicker])
  const [conversations, setConversations] = useState<api.Conversation[]>([])
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const listGeneration = useRef(0)
  const [selectedChat, setSelected] = useState<string>()
  const selected = workflow?.conversation.id ?? selectedChat
  const workflowVersion = workflow?.version
  const [referencedProject, setReferencedProject] = useState<{
    project: projectApi.Project
    version: number
  }>()
  const selectedRef = useRef<string | undefined>(undefined)
  selectedRef.current = selected
  const [history, setHistory] = useState<api.History>()
  const [verifiedHistory, setVerifiedHistory] = useState<string>()
  const [model, setModel] = useState("")
  const [efforts, setEfforts] = useState<Record<string, string>>({})
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const input = drafts[selected ?? ""] ?? ""
  const setInput = (value: string) =>
    setDrafts((current) => ({ ...current, [selected ?? ""]: value }))
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState("")
  const [listError, setListError] = useState(false)
  const [resourceVersion, setResourceVersion] = useState(0)
  const [delta, setDelta] = useState("")
  const [answers, setAnswers] = useState<Record<string, string>>({})
  const messagesView = useRef<HTMLDivElement>(null)
  const following = useRef(true)
  const shownConversation = useRef<string | undefined>(undefined)
  const shownContent = useRef("")
  const savedScroll = useRef<{ conversation: string | undefined; top: number }>(
    {
      conversation: undefined,
      top: 0,
    }
  )
  const restoreScroll = useRef(true)
  const [newMessages, setNewMessages] = useState(false)
  const pendingSends = useRef(new Map<string, PendingSend>())
  const pendingAnswers = useRef(
    new Map<string, { operationId: string; answer: string; sent: boolean }>()
  )
  const lifetime = useRef<AbortController | null>(null)
  const historyReader = useMemo(() => latestReader(api.read), [])
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  const userId = session.status === "active" ? session.session.user.id : ""
  const modelQuery = useQuery({
    ...modelQueryOptions(userId),
    enabled: available && Boolean(userId) && !workflow,
  })
  const models = modelQuery.data ?? []
  const selectedModel = models.find((item) => item.id === model)
  const effort = selectedEffort(selectedModel, efforts[model] ?? "")
  // A refetch can retain the previous failure while recovering successfully.
  const modelError = modelQuery.isError && modelQuery.fetchStatus === "idle"
  const missingProviders = modelQuery.data
    ? connectedProviders.filter(
        (provider) =>
          !models.some((item) => item.id.startsWith(`factory:${provider.id}:`))
      )
    : []
  useEffect(() => {
    // Disabling a query alone does not stop a retry already in progress.
    if (!available)
      void queryClient.cancelQueries({ queryKey: modelQueryKey(userId) })
  }, [available, queryClient, userId])
  const acceptHistory = useCallback((value: api.History) => {
    for (const [id, pending] of pendingAnswers.current) {
      if (pending.sent && !value.requests.some((request) => request.id === id))
        pendingAnswers.current.delete(id)
    }
    setHistory(value)
  }, [])
  const workflowRefresh = useRef(workflow?.onRefresh)
  workflowRefresh.current = workflow?.onRefresh
  useEffect(() => {
    if (history?.conversation.id === selected) workflowRefresh.current?.()
  }, [history, selected])

  const report = useCallback(
    (error: unknown, showMessage = true) => {
      if (
        error instanceof ApiFailure &&
        (error.status === 401 || error.status === 403)
      ) {
        sessionController.authorizationFailed()
        return
      }
      if (!showMessage) return
      setMessage(
        error instanceof api.ContextTransferFailure
          ? "This conversation is too long to transfer. Start a new chat to use another provider."
          : error instanceof ApiFailure && error.status === 409
            ? "The conversation changed or is still starting. Check its status before trying again."
            : "The conversation could not be updated. Check its status before trying again."
      )
    },
    [sessionController]
  )

  useEffect(() => {
    const controller = new AbortController()
    lifetime.current = controller
    return () => {
      controller.abort()
      lifetime.current = null
    }
  }, [])
  useEffect(() => {
    if (!available || !csrf) return
    const controller = new AbortController()
    const generation = ++listGeneration.current
    void api
      .list(controller.signal)
      .then((page) => {
        if (controller.signal.aborted || generation !== listGeneration.current)
          return
        setConversations((existing) => {
          const selectedItem = existing.find(
            (item) => item.id === selectedRef.current
          )
          return selectedItem &&
            !page.items.some((item) => item.id === selectedItem.id)
            ? [...page.items, selectedItem]
            : page.items
        })
        setNextCursor(page.nextCursor)
        setSelected((current) => current ?? page.items[0]?.id)
        setListError(false)
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted || generation !== listGeneration.current)
          return
        setListError(true)
        report(error, false)
      })
    return () => {
      controller.abort()
      listGeneration.current++
    }
  }, [available, csrf, report, resourceVersion])
  useEffect(() => {
    if (!modelQuery.data) return
    setModel((current) =>
      modelQuery.data.some((item) => item.id === current)
        ? current
        : (modelQuery.data[0]?.id ?? "")
    )
  }, [modelQuery.data])
  useEffect(() => {
    if (modelQuery.error) report(modelQuery.error, false)
  }, [modelQuery.error, report])
  useEffect(() => {
    if (!selected || !available) return
    const controller = new AbortController()
    void historyReader
      .read(selected, controller.signal)
      .then((value) => {
        if (value) {
          setDelta("")
          acceptHistory(value)
          setVerifiedHistory(selected)
        }
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) report(error)
      })
    return () => {
      controller.abort()
      historyReader.invalidate()
      setVerifiedHistory(undefined)
    }
  }, [
    selected,
    available,
    report,
    historyReader,
    acceptHistory,
    workflowVersion,
  ])
  const currentHistory =
    history?.conversation.id === selected ? history : undefined
  const activeRun = currentHistory?.runs
    .slice()
    .reverse()
    .find((run) => run.status === "active")
  // Reconcile the resumed snapshot before subscribing again. Otherwise its
  // completion can erase deltas that arrived on the new stream in the meantime.
  const activeId = verifiedHistory === selected ? activeRun?.id : undefined
  useEffect(() => {
    if (!selected || !activeId || !available) return
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout> | undefined
    const refresh = async () => {
      const value = await historyReader.read(selected, controller.signal)
      if (value) acceptHistory(value)
      return value
    }
    async function connect() {
      try {
        await api.stream(selected!, activeId!, controller.signal, (event) => {
          if (event.kind === "delta")
            setDelta((current) =>
              (current + (event.text ?? "")).slice(-256_000)
            )
          else {
            setDelta("")
            void refresh().catch((error: unknown) => {
              if (!controller.signal.aborted) report(error)
            })
          }
        })
        if (!controller.signal.aborted) await refresh()
      } catch (error) {
        if (controller.signal.aborted) return
        if (
          error instanceof ApiFailure &&
          (error.status === 401 || error.status === 403)
        ) {
          report(error)
          return
        }
        try {
          await refresh()
        } catch (refreshError) {
          if (!controller.signal.aborted) report(refreshError)
        }
      }
      if (!controller.signal.aborted)
        timer = setTimeout(() => {
          void connect()
        }, 1500)
    }
    void connect()
    return () => {
      controller.abort()
      clearTimeout(timer)
    }
  }, [selected, activeId, available, report, historyReader, acceptHistory])

  async function reload(id = selected) {
    const signal = lifetime.current?.signal
    if (
      !id ||
      id !== selectedRef.current ||
      !signal ||
      signal.aborted ||
      !available ||
      sessionController.getSnapshot().status !== "active"
    )
      return
    const value = await historyReader.read(id, signal)
    if (value && selectedRef.current === id) {
      if (verifiedHistory !== id) setDelta("")
      acceptHistory(value)
      setVerifiedHistory(id)
    }
  }
  async function perform(action: () => Promise<void>) {
    if (
      busy ||
      !csrf ||
      !lifetime.current ||
      sessionController.getSnapshot().status !== "active"
    )
      return
    const signal = lifetime.current.signal
    const origin = selectedRef.current
    setBusy(true)
    setMessage("")
    try {
      await action()
    } catch (error) {
      if (!signal.aborted) report(error, selectedRef.current === origin)
    } finally {
      if (!signal.aborted) setBusy(false)
    }
  }
  async function loadOlder() {
    const signal = lifetime.current?.signal
    const cursor = nextCursor
    if (
      !signal ||
      signal.aborted ||
      !cursor ||
      loadingMore ||
      !available ||
      sessionController.getSnapshot().status !== "active"
    )
      return
    const generation = listGeneration.current
    setLoadingMore(true)
    try {
      const page = await api.list(signal, cursor)
      if (signal.aborted || generation !== listGeneration.current) return
      setConversations((items) => mergeOlderConversations(items, page.items))
      setNextCursor(page.nextCursor)
      setListError(false)
    } catch (error) {
      if (!signal.aborted && generation === listGeneration.current) {
        setListError(true)
        report(error, false)
      }
    } finally {
      if (!signal.aborted) setLoadingMore(false)
    }
  }
  function newConversation(projectId?: string) {
    void perform(async () => {
      const created = await api.create(
        csrf,
        lifetime.current!.signal,
        projectId
      )
      if (!lifetime.current || lifetime.current.signal.aborted) return
      setConversations((items) => [created, ...items])
      setSelected(created.id)
      onOpenChat?.()
      setDelta("")
      setAnswers({})
    })
  }
  function send() {
    if (
      !selected ||
      !currentHistory ||
      verifiedHistory !== selected ||
      !input.trim() ||
      (!workflow && !selectedModel) ||
      activeRun ||
      uncertain ||
      !available
    )
      return
    const previous = pendingSends.current.get(selected)
    const sendModel = workflow ? "" : model
    const sendEffort = workflow ? "" : effort
    const operation =
      previous &&
      previous.conversation === selected &&
      previous.input === input &&
      previous.model === sendModel &&
      previous.effort === sendEffort
        ? previous
        : {
            operationId: crypto.randomUUID(),
            input,
            model: sendModel,
            effort: sendEffort,
            conversation: selected,
          }
    pendingSends.current.set(selected, operation)
    void perform(async () => {
      if (workflow)
        await workflow.start(
          operation.operationId,
          operation.input,
          csrf,
          lifetime.current!.signal
        )
      else
        await api.start(
          selected,
          {
            operationId: operation.operationId,
            input: operation.input,
            model: operation.model,
            effort: operation.effort || null,
          },
          csrf,
          lifetime.current!.signal
        )
      if (!lifetime.current || lifetime.current.signal.aborted) return
      pendingSends.current.delete(selected)
      setInput("")
      await reload(selected)
    })
  }
  function stop() {
    if (!selected || !activeRun) return
    void perform(async () => {
      await api.cancel(selected, activeRun.id, csrf, lifetime.current!.signal)
      await reload()
    })
  }
  function respond(request: api.History["requests"][number]) {
    if (!selected) return
    const values = request.questions.map((q) => ({
      id: q.id,
      value: answers[q.id] ?? "",
    }))
    if (values.some((answer) => !answer.value.trim())) {
      setMessage("Answer each question before submitting.")
      return
    }
    const serialized = JSON.stringify(values)
    const previous = pendingAnswers.current.get(request.id)
    if (previous && previous.answer !== serialized) {
      setMessage(
        "The previous answer may have been sent. Check its status before changing it."
      )
      return
    }
    const operation = previous ?? {
      operationId: crypto.randomUUID(),
      answer: serialized,
      sent: false,
    }
    pendingAnswers.current.set(request.id, operation)
    void perform(async () => {
      await api.answer(
        selected,
        request.runId,
        request.id,
        values,
        operation.operationId,
        csrf,
        lifetime.current!.signal
      )
      operation.sent = true
      await reload()
    })
  }
  const latestMessage =
    currentHistory?.messages[currentHistory.messages.length - 1]
  const requestVersion =
    currentHistory?.requests
      .map((request) => `${request.id}:${request.state}`)
      .join(",") ?? ""
  const contentVersion = `${currentHistory?.messages.length ?? 0}:${latestMessage?.id ?? ""}:${latestMessage?.content.length ?? 0}:${requestVersion}:${delta}`
  const viewport = useRef({
    selected,
    historyId: history?.conversation.id,
    contentVersion,
  })
  viewport.current = {
    selected,
    historyId: history?.conversation.id,
    contentVersion,
  }
  const positionMessages = useCallback((element: HTMLDivElement) => {
    const current = viewport.current
    if (!current.selected || current.historyId !== current.selected) {
      restoreScroll.current = true
      return
    }
    // A hidden mobile pane has no usable geometry. Observe its reveal before
    // marking content seen, without changing a reader's manual scroll choice.
    if (!element.clientHeight) return
    if (shownConversation.current !== current.selected) {
      shownConversation.current = current.selected
      following.current = true
    }
    if (following.current) {
      element.scrollTop = element.scrollHeight
      setNewMessages(false)
    } else {
      if (
        restoreScroll.current &&
        savedScroll.current.conversation === current.selected
      )
        element.scrollTop = savedScroll.current.top
      if (shownContent.current !== current.contentVersion) setNewMessages(true)
    }
    restoreScroll.current = false
    shownContent.current = current.contentVersion
  }, [])
  const observeMessages = useCallback(
    (element: HTMLDivElement | null) => {
      messagesView.current = element
      if (!element) return
      restoreScroll.current = true
      const observer = new ResizeObserver(() => positionMessages(element))
      observer.observe(element)
      positionMessages(element)
      return () => {
        if (element.clientHeight)
          savedScroll.current = {
            conversation: shownConversation.current,
            top: element.scrollTop,
          }
        observer.disconnect()
        messagesView.current = null
      }
    },
    [positionMessages]
  )
  useLayoutEffect(() => {
    if (messagesView.current) positionMessages(messagesView.current)
  }, [selected, history?.conversation.id, contentVersion, positionMessages])

  function onMessageScroll() {
    const element = messagesView.current
    if (!element?.clientHeight || !currentHistory) return
    savedScroll.current = { conversation: selected, top: element.scrollTop }
    following.current =
      element.scrollHeight - element.clientHeight - element.scrollTop <= 48
    if (following.current) setNewMessages(false)
  }
  function showLatest() {
    const element = messagesView.current
    if (!element) return
    following.current = true
    element.scrollTop = element.scrollHeight
    setNewMessages(false)
  }
  const uncertain = currentHistory?.runs
    .slice()
    .reverse()
    .find(
      (run) =>
        run.status === "uncertain" ||
        (!available && !resuming && run.status === "active")
    )
  const selectedConversation =
    workflow?.conversation ?? conversations.find((item) => item.id === selected)
  const projectId = selectedConversation?.projectId ?? undefined
  const listedProject = projectWorkspace.projects.find(
    (item) => item.id === projectId
  )
  const project =
    listedProject ??
    (referencedProject &&
    referencedProject.project.id === projectId &&
    referencedProject.version === projectWorkspace.refreshVersion
      ? referencedProject.project
      : undefined)
  useEffect(() => {
    if (listedProject?.id)
      projectWorkspace.setSelectedProjectId(listedProject.id)
  }, [listedProject?.id, projectWorkspace.setSelectedProjectId])
  const projectVerification = useProjectVerification(
    projectId,
    available &&
      projectWorkspace.connection?.status !== "disconnected" &&
      project?.state === "ready",
    selected
  )
  const warnings = projectWarnings(
    projectId,
    projectVerification,
    project?.renamedSinceClone ?? false
  )
  useEffect(() => {
    if (!projectId || listedProject || !available) return
    const controller = new AbortController()
    void projectApi
      .get(projectId, controller.signal)
      .then((value) => {
        if (!controller.signal.aborted)
          setReferencedProject({
            project: value,
            version: projectWorkspace.refreshVersion,
          })
      })
      .catch(() => {
        if (!controller.signal.aborted) setReferencedProject(undefined)
      })
    return () => controller.abort()
  }, [projectId, listedProject, available, projectWorkspace.refreshVersion])
  const title = selectedConversation
    ? `${project?.fullName ? `${project.fullName} · ` : ""}${conversationLabel(selectedConversation)}`
    : m.new_chat()
  const outcome = resuming
    ? m.checking_workspace()
    : translateMessage(runOutcome(currentHistory, available))
  const latestRun = currentHistory?.runs[currentHistory.runs.length - 1]
  const historyReady = Boolean(currentHistory) && verifiedHistory === selected
  const submitDisabled =
    !selected ||
    !historyReady ||
    (!workflow && !selectedModel) ||
    !input.trim() ||
    busy ||
    !available ||
    Boolean(uncertain)
  useEffect(() => {
    if (!workflow?.focusOnReady || !historyReady || !available) return
    const textarea = messagesView.current
      ?.closest(".chat")
      ?.querySelector("textarea")
    if (!textarea) return
    textarea.focus()
    workflow.onFocused?.()
  }, [workflow?.focusOnReady, workflow?.onFocused, historyReady, available])
  useEffect(() => {
    setMessage("")
    setDelta("")
    setAnswers({})
  }, [selected])

  function selectConversation(id: string) {
    onOpenChat?.()
    if (id === selected && !workflow) return
    setMessage("")
    setDelta("")
    setAnswers({})
    setSelected(id)
  }
  function retryList() {
    setResourceVersion((value) => value + 1)
  }
  function retryHistory() {
    setMessage("")
    void reload().catch(report)
  }

  return renderLayout({
    title,
    conversationId: selected,
    projectName: project?.fullName,
    projectWarnings: warnings,
    navigation: (
      <div className="workspace-navigation">
        <Button
          variant="ghost"
          className="conversation-new"
          onClick={() => newConversation()}
          disabled={!available || busy}
        >
          <SquarePen aria-hidden="true" size={16} />
          {m.new_chat()}{" "}
        </Button>
        <ProjectNavigation
          workspace={projectWorkspace}
          available={available}
          busy={busy}
          onNewChat={newConversation}
          conversations={conversations}
          selectedConversation={
            workflow || !navigationActive ? undefined : selected
          }
          onSelectConversation={selectConversation}
          onProjectRemoved={(id) => {
            const removed = conversations.filter(
              (item) => item.projectId === id
            )
            setConversations((items) =>
              items.filter((item) => item.projectId !== id)
            )
            if (removed.some((item) => item.id === selectedRef.current)) {
              setSelected(undefined)
              setHistory(undefined)
              setVerifiedHistory(undefined)
              setInput("")
            }
          }}
        />
        <ConversationNavigation
          conversations={conversations.filter(
            (conversation) =>
              !conversation.projectId ||
              !projectWorkspace.projects.some(
                (project) => project.id === conversation.projectId
              )
          )}
          selected={workflow || !navigationActive ? undefined : selected}
          available={available}
          busy={busy}
          hasOlder={Boolean(nextCursor)}
          loadingMore={loadingMore}
          failed={listError}
          onSelect={selectConversation}
          onLoadOlder={() => void loadOlder()}
          onRetry={retryList}
        />
      </div>
    ),
    conversation: (
      <section
        className="chat"
        aria-label={m.assistant_conversation()}
        aria-busy={resuming}
      >
        {warnings.map((warning) => (
          <p key={warning} className="chat-project-warning" role="status">
            {translateMessage(warning)}
          </p>
        ))}
        <div
          className="chat-messages"
          ref={observeMessages}
          role="region"
          tabIndex={0}
          aria-label={m.conversation_messages()}
          onScroll={onMessageScroll}
        >
          <div className="chat-transcript">
            {!selected && (
              <div className="chat-empty">
                <h2>{m.what_are_we_building()}</h2>
                <Button
                  variant="primary"
                  onClick={() => newConversation()}
                  disabled={!available || busy}
                >
                  {m.new_chat()}{" "}
                </Button>
              </div>
            )}
            {selected && !currentHistory && !message && available && (
              <LoadingState title={m.loading_chat()} />
            )}
            {selected && currentHistory?.messages.length === 0 && !delta && (
              <div className="chat-empty">
                <h2>{m.what_are_we_building()}</h2>
              </div>
            )}
            {currentHistory &&
              (currentHistory.runsTruncated ||
                currentHistory.messagesTruncated ||
                currentHistory.requestsTruncated) && (
                <p className="chat-history-note" role="status">
                  {m.only_recent_history_is_shown_older_items_are_not_displayed(
                    {
                      items: new Intl.ListFormat(locale, {
                        type: "disjunction",
                      }).format([
                        ...(currentHistory.runsTruncated ? [m.runs()] : []),
                        ...(currentHistory.messagesTruncated
                          ? [m.messages()]
                          : []),
                        ...(currentHistory.requestsTruncated
                          ? [m.requests()]
                          : []),
                      ]),
                    }
                  )}
                </p>
              )}
            {currentHistory?.messages.map((item) => (
              <article
                className={`chat-message chat-message-${item.role}`}
                key={item.id}
                aria-label={
                  item.role === "user"
                    ? m.your_message()
                    : m.assistant_message()
                }
              >
                {item.role === "assistant" && <strong>Assistant</strong>}
                {item.role === "assistant" ? (
                  <MessageMarkdown>{item.content}</MessageMarkdown>
                ) : (
                  <p>{item.content}</p>
                )}
              </article>
            ))}
            {delta && currentHistory && (
              <article
                className="chat-message chat-message-assistant"
                aria-label={m.assistant_response_in_progress()}
              >
                <strong>Assistant</strong>
                <MessageMarkdown previews={false}>{delta}</MessageMarkdown>
              </article>
            )}
            {currentHistory?.requests.map((request) => {
              const receipt = questionReceipt(
                request.state,
                pendingAnswers.current.get(request.id)?.sent ?? false
              )
              return (
                <form
                  className="chat-questions"
                  key={request.id}
                  onSubmit={(event) => {
                    event.preventDefault()
                    respond(request)
                  }}
                >
                  <h2>{m.assistant_has_a_question()}</h2>
                  {receipt.message && (
                    <p role="status">{translateMessage(receipt.message)}</p>
                  )}
                  {request.questions.map((question) => (
                    <fieldset
                      key={question.id}
                      disabled={receipt.disabled || busy || !available}
                    >
                      <legend>
                        {question.header}: {question.question}
                      </legend>
                      {question.options.map((option) => (
                        <label
                          className="chat-question-choice"
                          key={option.label}
                        >
                          <input
                            type="radio"
                            name={question.id}
                            value={option.label}
                            checked={answers[question.id] === option.label}
                            onChange={() =>
                              setAnswers((state) => ({
                                ...state,
                                [question.id]: option.label,
                              }))
                            }
                          />
                          <span>
                            {option.label}
                            <small>{option.description}</small>
                          </span>
                        </label>
                      ))}
                      {question.allowFreeText && (
                        <label className="chat-free-answer">
                          {m.other_answer()}{" "}
                          <input
                            value={
                              question.options.some(
                                (option) =>
                                  option.label === answers[question.id]
                              )
                                ? ""
                                : (answers[question.id] ?? "")
                            }
                            onChange={(event) =>
                              setAnswers((state) => ({
                                ...state,
                                [question.id]: event.target.value,
                              }))
                            }
                          />
                        </label>
                      )}
                    </fieldset>
                  ))}
                  <Button
                    type="submit"
                    disabled={receipt.disabled || busy || !available}
                  >
                    {m.send_answers()}{" "}
                  </Button>
                </form>
              )
            })}
          </div>
        </div>
        {newMessages && (
          <Button
            variant="secondary"
            className="chat-latest"
            onClick={showLatest}
          >
            <ArrowDown aria-hidden="true" size={14} />
            {m.jump_to_latest()}{" "}
          </Button>
        )}
        <div className="chat-bottom">
          {!workflow &&
            modelQuery.failureCount > 5 &&
            !modelError &&
            !models.length && (
              <p className="chat-status" role="status">
                {m.model_discovery_is_still_busy_retrying_automatically()}{" "}
              </p>
            )}
          {!workflow && modelError && (
            <div className="chat-notice" role="status">
              <p>
                {models.length
                  ? m.models_could_not_be_updated_your_previous_selection_remains_avail()
                  : m.models_could_not_be_loaded_existing_chats_remain_available()}
              </p>
              <Button
                variant="ghost"
                disabled={!available || modelQuery.isFetching}
                onClick={() =>
                  void modelQuery.refetch({ cancelRefetch: false })
                }
              >
                {m.retry_models()}{" "}
              </Button>
            </div>
          )}
          {!workflow && !modelError && missingProviders.length > 0 && (
            <div className="chat-notice" role="status">
              <p>
                {m.models_from_providers_could_not_be_loaded({
                  providers: missingProviders
                    .map((provider) => provider.name)
                    .join(", "),
                })}
              </p>
              <Button
                variant="ghost"
                disabled={!available || modelQuery.isFetching}
                onClick={() =>
                  void modelQuery.refetch({ cancelRefetch: false })
                }
              >
                {m.retry_models()}{" "}
              </Button>
            </div>
          )}
          {message && (
            <div className="chat-notice" role="status">
              <p>{translateMessage(message)}</p>
              {selected && (
                <Button
                  variant="ghost"
                  disabled={!available || busy}
                  onClick={retryHistory}
                >
                  {m.check_status()}{" "}
                </Button>
              )}
            </div>
          )}
          {uncertain && (
            <p className="chat-notice" role="status">
              {m.this_run_is_not_confirmed_stopped_use_stop_environment_in_your_pr()}{" "}
            </p>
          )}
          {outcome && !uncertain && (
            <p
              className={
                resuming || latestRun?.status === "completed"
                  ? "sr-only"
                  : "chat-status"
              }
              role="status"
              aria-live="polite"
            >
              {outcome}
            </p>
          )}
          <ChatComposer
            input={input}
            model={model}
            models={models}
            effort={effort}
            disabled={!selected || !historyReady || !available || busy}
            sendDisabled={submitDisabled}
            running={Boolean(activeRun)}
            stopDisabled={busy || !available}
            onInput={setInput}
            onModel={setModel}
            onEffort={(value) =>
              setEfforts((current) => ({ ...current, [model]: value }))
            }
            onSend={send}
            onStop={stop}
            workflow={
              workflow
                ? {
                    modelLabel: workflow.modelLabel,
                    access: m.feature_access(),
                  }
                : undefined
            }
          />
        </div>
      </section>
    ),
  })
}
