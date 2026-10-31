import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"

import { useQuery } from "@tanstack/react-query"

import { ApiFailure } from "../../lib/api/client"
import { useSession } from "../../lib/auth/session-context"
import * as api from "./api"
import { modelQueryOptions } from "./model-query"
import {
  latestReader,
  runOutcome,
  questionReceipt,
  mergeOlderConversations,
} from "./state"
import "./conversations.css"

type PendingSend = {
  operationId: string
  input: string
  model: string
  conversation: string
}

export function Chat({ available }: { available: boolean }) {
  const { state: session, controller: sessionController } = useSession()
  const [conversations, setConversations] = useState<api.Conversation[]>([])
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const listGeneration = useRef(0)
  const [selected, setSelected] = useState<string>()
  const selectedRef = useRef<string | undefined>(undefined)
  selectedRef.current = selected
  const [history, setHistory] = useState<api.History>()
  const [model, setModel] = useState("")
  const [input, setInput] = useState("")
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
  const [newMessages, setNewMessages] = useState(false)
  const pendingSend = useRef<PendingSend | null>(null)
  const pendingAnswers = useRef(
    new Map<string, { operationId: string; answer: string; sent: boolean }>()
  )
  const lifetime = useRef<AbortController | null>(null)
  const historyReader = useMemo(() => latestReader(api.read), [])
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  const userId = session.status === "active" ? session.session.user.id : ""
  const modelQuery = useQuery({
    ...modelQueryOptions(userId),
    enabled: available && Boolean(userId),
  })
  const models = modelQuery.data ?? []
  const modelError = modelQuery.isError
  const acceptHistory = useCallback((value: api.History) => {
    for (const [id, pending] of pendingAnswers.current) {
      if (pending.sent && !value.requests.some((request) => request.id === id))
        pendingAnswers.current.delete(id)
    }
    setHistory(value)
  }, [])

  const report = useCallback(
    (error: unknown) => {
      if (
        error instanceof ApiFailure &&
        (error.status === 401 || error.status === 403)
      ) {
        sessionController.authorizationFailed()
        return
      }
      setMessage(
        error instanceof ApiFailure && error.status === 409
          ? "The conversation changed or is still starting. Refresh its status."
          : "The conversation could not be updated. Check the connection and refresh."
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
        report(error)
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
    if (modelQuery.error) report(modelQuery.error)
  }, [modelQuery.error, report])
  useEffect(() => {
    if (!selected || !available) return
    const controller = new AbortController()
    setHistory(undefined)
    setDelta("")
    setAnswers({})
    void historyReader
      .read(selected, controller.signal)
      .then((value) => {
        if (value) acceptHistory(value)
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) report(error)
      })
    return () => {
      controller.abort()
      historyReader.invalidate()
    }
  }, [selected, available, report, historyReader, acceptHistory])
  const activeRun = history?.runs
    .slice()
    .reverse()
    .find((run) => run.status === "active")
  const activeId = activeRun?.id
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
    if (!id || !signal || signal.aborted || !available) return
    const value = await historyReader.read(id, signal)
    if (value) acceptHistory(value)
  }
  async function perform(action: () => Promise<void>) {
    if (busy || !csrf || !lifetime.current) return
    const signal = lifetime.current.signal
    setBusy(true)
    setMessage("")
    try {
      await action()
    } catch (error) {
      if (!signal.aborted) report(error)
    } finally {
      if (!signal.aborted) setBusy(false)
    }
  }
  async function loadOlder() {
    const signal = lifetime.current?.signal
    const cursor = nextCursor
    if (!signal || signal.aborted || !cursor || loadingMore || !available)
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
        report(error)
      }
    } finally {
      if (!signal.aborted) setLoadingMore(false)
    }
  }
  function newConversation() {
    void perform(async () => {
      const created = await api.create(csrf, lifetime.current!.signal)
      if (!lifetime.current || lifetime.current.signal.aborted) return
      setConversations((items) => [created, ...items])
      setSelected(created.id)
      setInput("")
      pendingSend.current = null
    })
  }
  function send() {
    if (!selected || !input.trim() || !model || activeRun || !available) return
    const previous = pendingSend.current
    const operation =
      previous &&
      previous.conversation === selected &&
      previous.input === input &&
      previous.model === model
        ? previous
        : {
            operationId: crypto.randomUUID(),
            input,
            model,
            conversation: selected,
          }
    pendingSend.current = operation
    void perform(async () => {
      await api.start(
        selected,
        {
          operationId: operation.operationId,
          input: operation.input,
          model: operation.model,
        },
        csrf,
        lifetime.current!.signal
      )
      if (!lifetime.current || lifetime.current.signal.aborted) return
      pendingSend.current = null
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
        "The previous answer may have been sent. Refresh its status before changing it."
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
  const latestMessage = history?.messages[history.messages.length - 1]
  const contentVersion = `${history?.messages.length ?? 0}:${latestMessage?.id ?? ""}:${latestMessage?.content.length ?? 0}:${delta}`
  useLayoutEffect(() => {
    const element = messagesView.current
    if (!element || !selected || history?.conversation.id !== selected) return
    if (shownConversation.current !== selected) {
      shownConversation.current = selected
      shownContent.current = contentVersion
      following.current = true
      setNewMessages(false)
      element.scrollTop = element.scrollHeight
      return
    }
    if (shownContent.current === contentVersion) return
    shownContent.current = contentVersion
    if (following.current) {
      element.scrollTop = element.scrollHeight
      setNewMessages(false)
    } else {
      setNewMessages(true)
    }
  }, [selected, history?.conversation.id, contentVersion])

  function onMessageScroll() {
    const element = messagesView.current
    if (!element) return
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
  const uncertain = history?.runs
    .slice()
    .reverse()
    .find(
      (run) =>
        run.status === "uncertain" || (!available && run.status === "active")
    )
  return (
    <section className="chat" aria-labelledby="chat-title">
      <div className="chat-heading">
        <div>
          <p className="chat-kicker">Workspace</p>
          <h1 id="chat-title">Codex</h1>
        </div>
        <button
          type="button"
          onClick={newConversation}
          disabled={!available || busy}
        >
          New chat
        </button>
      </div>
      <p className="chat-access">
        Full access: Codex can read, change or delete files and credentials
        inside your environment without asking for each action.
      </p>
      <div className="chat-toolbar">
        <label>
          Conversation{" "}
          <select
            value={selected ?? ""}
            onChange={(event) => {
              pendingSend.current = null
              setSelected(event.target.value || undefined)
            }}
            disabled={busy || !available}
          >
            <option value="">Select a conversation</option>
            {conversations.map((item, index) => (
              <option key={item.id} value={item.id}>
                Chat {conversations.length - index} ·{" "}
                {new Date(item.createdAt).toLocaleString()}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          disabled={!selected || !available || busy}
          onClick={() => {
            void reload().catch(report)
          }}
        >
          Refresh
        </button>
      </div>
      {nextCursor && (
        <button
          type="button"
          disabled={!available || loadingMore}
          onClick={() => void loadOlder()}
        >
          {loadingMore
            ? "Loading older conversations…"
            : "Load older conversations"}
        </button>
      )}
      {(listError || modelError) && (
        <div className="chat-notice">
          <p>
            {listError && "Conversation list is unavailable. "}
            {modelError &&
              "Model list is unavailable; existing conversations remain accessible. Retry, or stop the environment and restore model access if the issue persists."}
          </p>
          <button
            type="button"
            disabled={!available}
            onClick={() => {
              setMessage("")
              setResourceVersion((value) => value + 1)
              if (modelError) void modelQuery.refetch()
            }}
          >
            Retry workspace data
          </button>
        </div>
      )}
      <p role="status" aria-live="polite" className="chat-status">
        {[runOutcome(history, available), message].filter(Boolean).join(" ")}
      </p>
      {uncertain && (
        <div className="chat-notice">
          <p>
            The previous run has not been confirmed stopped. Stop the
            environment and check Recovery in Environment and account before
            starting another run. Some launches cannot be recovered
            automatically.
          </p>
        </div>
      )}
      {history &&
        (history.runsTruncated ||
          history.messagesTruncated ||
          history.requestsTruncated) && (
          <p className="chat-notice" role="status">
            Only recent conversation history is shown. Older{" "}
            {[
              history.runsTruncated && "runs",
              history.messagesTruncated && "messages",
              history.requestsTruncated && "requests",
            ]
              .filter(Boolean)
              .join(", ")}{" "}
            are not displayed.
          </p>
        )}
      <div
        className="chat-messages"
        ref={messagesView}
        role="region"
        tabIndex={0}
        aria-label="Conversation messages"
        onScroll={onMessageScroll}
      >
        {!selected && (
          <div className="chat-empty">
            <h2>Start a conversation</h2>
            <p>
              Create a chat, choose a model and describe what you want to work
              on.
            </p>
            <button
              type="button"
              disabled={!available || busy}
              onClick={newConversation}
            >
              New chat
            </button>
          </div>
        )}
        {selected && history && history.messages.length === 0 && !delta && (
          <div className="chat-empty">
            <h2>Ready when you are</h2>
            <p>Describe a task below to start working with Codex.</p>
          </div>
        )}
        {history?.messages.map((item) => (
          <article
            className={`chat-message chat-message-${item.role}`}
            key={item.id}
          >
            <strong>{item.role === "user" ? "You" : "Codex"}</strong>
            <p>{item.content}</p>
          </article>
        ))}
        {delta && (
          <article className="chat-message">
            <strong>Codex</strong>
            <p>{delta}</p>
          </article>
        )}
      </div>
      {(Boolean(history?.messages.length) || Boolean(delta)) && (
        <button
          type="button"
          className="chat-latest"
          aria-live="polite"
          onClick={showLatest}
        >
          {newMessages ? "New messages · Jump to latest" : "Jump to latest"}
        </button>
      )}
      {history?.requests.map((request) => {
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
            <h2>Codex has a question</h2>
            {receipt.message && <p role="status">{receipt.message}</p>}
            {request.questions.map((q) => (
              <fieldset
                key={q.id}
                disabled={receipt.disabled || busy || !available}
              >
                <legend>
                  {q.header}: {q.question}
                </legend>
                {q.options.map((option) => (
                  <label key={option.label}>
                    <input
                      type="radio"
                      name={q.id}
                      value={option.label}
                      checked={answers[q.id] === option.label}
                      onChange={() =>
                        setAnswers((state) => ({
                          ...state,
                          [q.id]: option.label,
                        }))
                      }
                    />
                    {option.label} <span>{option.description}</span>
                  </label>
                ))}
                {q.allowFreeText && (
                  <label>
                    Other answer{" "}
                    <input
                      value={
                        q.options.some((o) => o.label === answers[q.id])
                          ? ""
                          : (answers[q.id] ?? "")
                      }
                      onChange={(event) =>
                        setAnswers((state) => ({
                          ...state,
                          [q.id]: event.target.value,
                        }))
                      }
                    />
                  </label>
                )}
              </fieldset>
            ))}
            <button disabled={receipt.disabled || busy || !available}>
              Send answers
            </button>
          </form>
        )
      })}
      <div className="chat-composer">
        <label htmlFor="chat-input">Message Codex</label>
        <textarea
          id="chat-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          placeholder="Describe what you want to work on…"
          disabled={!selected || !available || busy}
          rows={3}
        />
        <div className="chat-composer-actions">
          <label>
            Model{" "}
            <select
              value={model}
              onChange={(event) => setModel(event.target.value)}
              disabled={!available || busy || !models.length}
            >
              {models.length === 0 && (
                <option value="">No models available</option>
              )}
              {models.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={() => void modelQuery.refetch()}
            disabled={!available || modelQuery.isFetching}
          >
            {modelQuery.isFetching ? "Refreshing models…" : "Refresh models"}
          </button>
          {activeRun ? (
            <button type="button" onClick={stop} disabled={busy || !available}>
              Stop
            </button>
          ) : (
            <button
              type="button"
              onClick={send}
              disabled={
                !selected ||
                !model ||
                !input.trim() ||
                busy ||
                !available ||
                Boolean(uncertain)
              }
            >
              Send
            </button>
          )}
        </div>
      </div>
    </section>
  )
}
