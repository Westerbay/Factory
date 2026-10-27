# Product

## Intended outcome

An operator will select a local workspace and skills, submit tasks, observe agents and
native delegation, answer questions emitted by skills, stop work and revisit history.
Skills define their questions and workflow gates; Factory will transport and display them.
A missing response will never be interpreted as approval.

## Current increment

Opening Factory shows a minimal desktop foundation screen. It explicitly states that
no agent is running and no database has been opened. There are no simulated execution
controls, invented running agents or sample history.

## Implemented execution rules

The application layer reserves one active root run, observes real-agent parent/child
identity and scopes blocking questions to their originating execution. Nonblocking
questions do not suspend it. Structured answers and provider permission decisions are
separate, explicit, single-use operations. Ordinary prose never creates a blocking
question. An explicit reply to a terminal conversation starts a new linked run, leaving
the previous result intact and retaining the one-root limit.

Only confirmed provider success can complete a run. Stop enters cancelling before
confirmed cleanup; disconnection invalidates outstanding requests. A failed storage
operation blocks further commands and initiates owned-runtime cleanup. Failure to
confirm cleanup remains visible. These rules currently run only in deterministic tests;
they are not desktop controls or a real-agent demonstration.

## Following steps

PostgreSQL persistence preserves execution history and completed messages. Explicit
startup recovery interrupts unfinished runs and invalidates pending interaction handles;
it never restarts tools. The persistence adapter is verified against isolated PostgreSQL,
and is not yet connected to the window.

The remaining sequence adds local runtime/desktop communication,
the Codex adapter, the operator interface and a bounded real-agent acceptance journey.
Those integrations are not connected yet. Cross-provider orchestration,
remote hosting, skill authoring and an inherited template visual identity are excluded.
