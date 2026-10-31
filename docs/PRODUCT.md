# Product

## Current capabilities

Factory offers a private member space and an administrator backoffice. Sign-in uses
GitHub only; there are no passwords or email invitations. A successful GitHub identity
check does not itself grant membership. Connecting Codex is a separate explicit action
inside an active member's private environment.

An operator configures the first administrator with the local CLI, selecting a GitHub
username and confirming the resolved profile. The stable provider ID is stored in the
database; web sign-in never creates an administrator. Setup refuses to replace an
existing administrator.
Another verified account becomes pending without a member session. The administrator
can activate that account, after which the member must begin a new sign-in. Handles and
display names identify people visually but never determine authorization.

The member workspace shows the user's environment and Codex account. Once connected,
it offers Codex conversations and a terminal alongside them. Repository selection,
cloning and global skills are not available.
GitHub sign-in does not request repository permissions or save a provider access token.

## Private local environments

Creating or activating an account creates no Docker resources. On first entry to the
workspace, an active member automatically requests an environment if no record exists;
that creates a private home volume. An existing stopped environment requires an explicit
Start. Files under `/home/factory`, including
the working directory `/home/factory/Documents`, survive stop/start, sign-out and
suspension. There is no volume deletion or reset action. A missing previously recorded
volume produces an error rather than an empty replacement.

Start and stop requests are asynchronous. Running and Stopped reflect successful runtime
inspection, with the last observation shown; the status API alone never starts an
environment. The workspace initializes only a never-created environment. Users cannot target another user's environment. The administrator's
access controls do not provide environment browsing or lifecycle controls for others.

Environments run independently of browser presence. Closing the page, going offline,
signing out or reaching session expiry does not stop them. Opening Factory the next day
reads their current state without renewing or recreating them. Only an explicit stop or
account suspension records a stop request. Suspension immediately revokes access; actual
container shutdown follows asynchronously while the API and Docker are available.
API shutdown does not stop containers, and restarting the API resumes supervision.
This does not guarantee task completion across host outages or process failures.

Factory imposes no limit on the number of started environments, CPU, memory or process
counts. They share available host resources; Docker Desktop and operating-system limits
still apply. Containers retain a non-root user and a read-only root
filesystem. Temporary storage and logs remain bounded; persistent named volumes have no
portable disk quota. Each user has an isolated internal network; only approved Codex
HTTPS destinations are reachable through the shared installation proxy. General Internet,
host/LAN and other-user access are blocked. There is no file editor.

Existing environments with a different image or policy are never silently replaced.
A local database reset removes database records but does not delete Docker homes.

## Terminal and personal Codex account

The terminal opens in `~/Documents` with zsh and Oh My Zsh in the current image,
and attaches to a persistent tmux shell. Closing the
page, losing the connection or restarting the API detaches the display without stopping
the shell. Stop environment ends its processes; saved files remain. One view owns input
at a time, and another view must explicitly take over. Reconnection never replays unsent
keystrokes. A slow display is detached while the shell continues to consume output.
Keyboard escape, screen-reader mode and mobile modifier keys are available.

Factory rechecks session access when returning to the page. This can close the displayed
terminal; use Open terminal again after verification to reattach. Offline or revoked
sessions hide private terminal and account data immediately in the browser. The server
checks input authorization and periodically closes passive revoked streams.

Connect Codex displays a short-lived authorization code and a validated official OpenAI
link. The member authorizes their personal ChatGPT/Codex account on that site; Factory
never asks for a password or API key. A code alone does not mean connected: the provider
must confirm completion and the account must be read successfully. Model access still
depends on the member's subscription and permissions. The connection can be cancelled;
only its initiating Factory session can see or cancel the pending code.

Credentials remain in that user's private container home. The browser holds account and
challenge information only in memory. Signing out of Factory does not delete the saved
Codex account. Disconnect Codex removes that saved sign-in while preserving projects;
it is blocked while a Factory conversation run is active or its cleanup is uncertain.
Disconnect does not guarantee interruption of Codex processes launched from the shell.
Stop environment is the explicit way to end all its processes. If an account controller
is abandoned during API failure, its watchdog bounds recovery to at most ten minutes.

A missing or stopped proxy makes Codex status/login unavailable. The local terminal
and local account cancellation/logout remain independent of proxy availability, subject
to the active-run guard. Members can also deliberately run Codex in the terminal.

## Codex conversations

After a confirmed Codex connection, a member can create and select conversations, choose
an available model, send messages, view streamed output, answer questions and request
cancellation. Completed messages and run states are retained for that user; reading
history does not replay a message or start tools. The conversation list loads older
pages on request. Each conversation shows a bounded recent history and indicates when older runs, messages or requests are omitted. A submitted
question answer can remain pending or uncertain until its receipt is confirmed; it is
not automatically resent. The model list is cached in browser memory for five minutes per user, with an explicit refresh; account and session changes clear it. A listed model is not a guarantee that the account can use it. Provider failures and interrupted or uncertain work remain visible.

Codex runs with full access inside the member's container: commands and file changes do
not ask for per-action approval, and the agent can read, change or delete any files
accessible to that container user, including the saved Codex credentials. Docker's
user isolation and network controls still apply, but do not protect a member's own home
from that member's agent. Treat repository content and external instructions accordingly.
Claude and multiple accounts per provider are not yet available.

An interrupt acknowledgement alone does not prove that all work stopped. If cleanup is
uncertain, the run remains blocking. Stop the environment, wait until the stopped state
is confirmed, and use the recovery control before starting it again. Recovery also
requires the process launch request to have conclusively settled. If its outcome is
unknown, the run can remain blocked even after Stop; no verified operator recovery is
available yet, and upgrading the container does not establish cleanup. Recovery releases
local execution ownership; it cannot undo effects already performed. Model discovery also
retains a reservation if its process cleanup is uncertain. A verified stopped observation
at a later environment generation releases that reservation automatically when its launch
request has settled, including for suspended members. An unsettled launch remains blocked. If model access remains unavailable after retrying, stop the environment
and use **Restore model access** to check recovery before starting it again. A real model turn and provider-specific behavior still require human
acceptance.

## Access and sessions

The administrator can browse bounded pages of pending, active and suspended users,
activate members, and suspend them after explicit confirmation. A stale edit refreshes
the list for review instead of overwriting a newer decision. There is no role editing,
second-administrator creation, user deletion or suspension of the administrator.

Suspension revokes all target sessions immediately on the server. Reactivation never
restores an old session and requires a new OAuth attempt started after activation.
Signing out revokes the current session. Sessions survive API restarts until their fixed
expiry, provided storage is available; activity does not silently extend their lifetime.

The server rechecks authorization for protected operations. The browser hides private
content during session verification, on connection failure and while offline. Sign-out
and account changes invalidate other open tabs. Private query data and provider credentials
are not persisted in localStorage, sessionStorage or IndexedDB. If sign-out cannot be
confirmed, private content stays hidden and the user can explicitly retry it.

## Installation and updates

The built frontend is installable in supported browsers. Its offline fallback is a public
page explaining that a connection is needed; installation does not make private work
available offline. Service-worker updates are checked on focus, reconnection and
periodically. A user chooses when to apply the available update and reload the current
tab; another tab's choice does not automatically reload it.

## Direction and preserved execution behavior

Future work will connect each user's isolated server environment to repository providers
and global skills. Providers should remain replaceable.
Repository authorization and agent execution remain explicit capabilities, never hidden
actions of sign-in.

The retained TypeScript execution implementation defines behavior to preserve during
porting: explicit task start and continuation, provider-observed parent/child agents,
independently answerable questions, explicit one-action permissions, cancellation with
confirmed cleanup, and durable history. Reading history never resumes tools. Silence,
timeouts or prose never grant approval. Only confirmed provider success completes a run;
a process exit or interrupt acknowledgement alone cannot prove descendant cleanup.

Those mechanisms currently reserve one global active root and do not implement web-user
ownership. They are reference code, not an available web capability. Historical reference
records are not assigned to web users; a database reset deletes them.
Live Codex execution and observed cleanup remain separate acceptance work.
