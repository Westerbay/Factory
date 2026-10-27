import { useState, type ChangeEvent, type FormEvent } from "react"
import type { RunProjection } from "@workspace/contracts"
import { operator } from "./model.js"
import { messages as m } from "./messages.js"
type Question = RunProjection["questions"][number]
type Permission = RunProjection["permissions"][number]
export function QuestionForm({
  runId,
  question,
  disabled,
}: {
  runId: string
  question: Question
  disabled: boolean
}) {
  const [choice, setChoice] = useState("")
  const [text, setText] = useState("")
  const submitting = question.status === "submitting"
  const unavailable = disabled || question.status !== "pending"
  function changeChoice(event: ChangeEvent<HTMLSelectElement>) {
    setChoice(event.target.value)
  }
  function changeText(event: ChangeEvent<HTMLTextAreaElement>) {
    setText(event.target.value)
  }
  function submit(event: FormEvent) {
    event.preventDefault()
    const answer = choice ? { optionId: choice } : { text }
    void operator.command(
      {
        type: "answer-question",
        runId,
        agentId: question.agentId,
        questionId: question.id,
        answer,
      },
      `question-${question.id}`
    )
  }
  const options = question.options.map((option) => (
    <option key={option.id} value={option.id}>
      {option.label}
    </option>
  ))
  return (
    <form onSubmit={submit} className="request">
      <h4>{question.prompt}</h4>
      <p className="meta">
        {question.agentId} · {question.blocking ? m.blocking : m.nonBlocking} ·{" "}
        {question.status}
      </p>
      <fieldset disabled={unavailable}>
        <legend>{m.answer}</legend>
        {question.options.length > 0 && (
          <label>
            {m.selectAnswer}
            <select value={choice} onChange={changeChoice}>
              <option value="">{m.selectAnswer}</option>
              {options}
            </select>
          </label>
        )}
        {question.allowFreeText && (
          <label>
            {m.freeText}
            <textarea
              value={text}
              onChange={changeText}
              disabled={Boolean(choice)}
              maxLength={65536}
            />
          </label>
        )}
        <button type="submit" disabled={!choice && !text.trim()}>
          {m.submit}
        </button>
      </fieldset>
      {submitting && <p role="status">{m.submitting}</p>}
    </form>
  )
}
export function PermissionForm({
  runId,
  request,
  disabled,
}: {
  runId: string
  request: Permission
  disabled: boolean
}) {
  function allow() {
    respond("allow")
  }
  function deny() {
    respond("deny")
  }
  function respond(decision: "allow" | "deny") {
    void operator.command(
      {
        type: "answer-permission",
        runId,
        agentId: request.agentId,
        requestId: request.id,
        decision,
      },
      `permission-${request.id}`
    )
  }
  const unavailable = disabled || request.status !== "pending"
  return (
    <section className="request">
      <h4>{request.description}</h4>
      <p className="meta">
        {request.agentId} · {request.status}
      </p>
      <div className="actions">
        <button onClick={allow} disabled={unavailable}>
          {m.allow}
        </button>
        <button onClick={deny} disabled={unavailable}>
          {m.deny}
        </button>
      </div>
    </section>
  )
}
