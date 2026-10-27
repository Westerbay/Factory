# Product

## Intended outcome

An operator will select a local workspace and skills, submit tasks, observe agents and
native delegation, answer questions emitted by skills, stop work and revisit history.
Skills define their questions and workflow gates; Factory will transport and display them.
A missing response will never be interpreted as approval.

## Current increment

Opening Factory starts one isolated utility host. It verifies the owned local PostgreSQL
identity, checks migrations and explicitly interrupts abandoned persisted work before
accepting commands. Startup never provisions, migrates or replays tools. Missing database
setup leaves the interface visibly unavailable with documented recovery steps.

The operator chooses a directory through a native dialog and optionally selects a
provider-discovered skill. Provider readiness is distinct from database/host readiness.
Tasks, hierarchy, inert plain-text output, independently submittable questions, explicit
one-action permissions, stop, active steering, linked terminal continuation and paged
persisted history are available. An empty history contains no example runs.

The skill list shows actual enabled names and paths returned by the runtime. The current
port has no description or disabled-entry metadata; the complete intended catalog is not
yet implemented. A capability flag describes protocol support, not live delegation proof.

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
confirm cleanup remains visible. These rules and the Codex protocol mappings are exercised with deterministic fixtures.
The built Electron journey uses a separate synthetic provider and actual owned fixture
PostgreSQL; it is not a live Codex demonstration.

## Following steps

PostgreSQL persistence preserves execution history and completed messages. Explicit
startup recovery interrupts unfinished runs and invalidates pending interaction handles;
it never restarts tools. The persistence adapter is verified against isolated PostgreSQL,
and is connected only through the privileged utility host.

The remaining sequence performs a bounded real-agent acceptance journey. Cross-provider orchestration,
remote hosting, skill authoring and an inherited template visual identity are excluded.

## Codex adapter limits

Codex structured requests remain distinct from prose and permission decisions. A provider
request containing multiple questions is answered once after all required answers arrive;
each question can submit independently while awaiting the shared provider acknowledgement.
Secret-input questions, file-change approvals without a supported patch presentation,
and unsupported actionable requests are rejected visibly. Silence,
timeouts and request invalidation never grant permission. Permission action details are
bounded; oversized details are rejected rather than truncated into uninformed consent.

Native agents are identified only from provider collaboration events. Success waits for
observed descendants to finish. Stop requires terminal confirmation for every owned thread;
a request acknowledgement or app-server exit alone is insufficient. Unconfirmed native
cleanup closes the owned app-server and remains a visible cancelling failure.
A provider disconnect can terminate the recorded execution while cleanup stays unconfirmed.
That outstanding ownership blocks new execution and makes shutdown fail visibly.
Native follow-up turns receive fresh core execution identities while retaining provider
thread ownership; terminal core agents are never reopened.
Conversation continuation is available within the current runtime session; provider IDs are
kept inside the adapter and cannot currently resume a conversation after host restart.

## Desktop safety and failure behavior

Every question and permission requires an explicit response. Submitting one question does
not disable another question while their shared provider response is incomplete. Reading
history or selecting an old run never executes or resumes it. Active steering and terminal
continuation are explicit separate actions.

Closing the native window asks whether to keep it open or stop and close. Successful close
waits for owned execution cleanup and the database pool. A failed close stays visible and
keeps the window open; cleanup is not inferred from utility-process death. Host loss blocks
new commands, clears temporary output and labels the visible projection as the last
persisted state. Restart recovery marks abandoned work interrupted without replaying it.

Transient output is bounded and may be coalesced under load; completed messages and durable
history remain authoritative. Rich Markdown, graph rendering, workflow editing, remote
hosting, installers and a fixed visual identity are outside this increment.

Execution history uses bounded pages of 50 runs with previous/next navigation; there is
no 200-run access limit. The selected detail and knowledge of the active root remain
independent of the visible page. Failed stop confirmation leaves an explicit retry
available once its pending request settles. Failed shutdown preserves persistence until
owned execution cleanup can be confirmed on an explicit retry; ordinary admission stays
closed throughout shutdown.

If execution cleanup succeeds but pool closure rejects, the host retains that specific
unconfirmed persistence-closure error. The current database adapter marks itself closed
before awaiting its underlying connection shutdown, so a second no-op cannot establish
confirmation. No normal work is readmitted; check local connections before quitting
externally or restarting. This is distinct from retryable execution cleanup failure.
