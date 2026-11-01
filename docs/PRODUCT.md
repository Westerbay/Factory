# Product

## Current capabilities

Factory offers a private member space and an administrator backoffice. Sign-in uses
GitHub only; there are no passwords or email invitations. A successful GitHub identity
check does not itself grant membership. Connecting an agent provider is a separate explicit action
inside an active member's private environment.

An operator configures the first administrator with the local CLI, selecting a GitHub
username and confirming the resolved profile. The stable provider ID is stored in the
database; web sign-in never creates an administrator. Setup refuses to replace an
existing administrator.
Another verified account becomes pending without a member session. The administrator
can activate that account, after which the member must begin a new sign-in. Handles and
display names identify people visually but never determine authorization.

After GitHub sign-in, workspace preparation starts automatically. An unconnected member
chooses Codex or Claude Code; a saved connected account opens the workspace
directly. The workspace has conversation history on the left, chat in the center and a
collapsible terminal on the right. The GitHub profile at the bottom left exposes Settings,
Stop environment and Sign out, plus User access for administrators. Provider account
details, additional connections and disconnect actions belong in Settings. Repository selection, cloning and global
skills are not available. The shared graphite interface uses Space Mono and concise
contextual status instead of an environment dashboard or permanent refresh controls.
GitHub sign-in does not request repository permissions or save a provider access token.

## Private local environments

Creating or activating an account creates no Docker resources. On entry to the workspace,
an active member automatically initializes a missing environment or starts a stopped one.
An existing running environment is reused. Before starting a stopped environment,
Factory checks for recoverable runs; a failed check or outstanding recovery keeps it
stopped with an explicit recovery path. Initialization creates a private home volume.
Files under `/home/factory`, including the working directory `/home/factory/Documents`,
survive stop/start, sign-out and suspension. There is no volume deletion or reset action.
A missing previously recorded volume produces an error rather than an empty replacement.

Start and stop requests are asynchronous. A loading screen covers startup; failures stay
visible with contextual retry. Running and Stopped require successful runtime inspection;
request acceptance alone proves neither. The status API never starts an environment.
Automatic entry is consumed once per open app session, including when the environment is
already running. Focus changes, route remounts and polling cannot reverse a subsequent
Stop. Reloading or opening a new tab is a new entry. **Return to workspace** explicitly
resumes a stopped workspace after required recovery. Users cannot target another user's
environment. Administrator access controls do not provide lifecycle controls for others.

Stop environment is in the profile menu and opens a confirmation explaining that running
tasks and terminal processes end while saved files and completed chat messages remain. Only
confirmation submits the stop request. Settings and Sign out remain reachable while the
environment is stopped.

Environments run independently of browser presence. Closing the page, going offline,
signing out or reaching session expiry does not stop them. Opening Factory the next day
reuses a running environment or attempts to start a stopped one. Only an explicit stop or
account suspension records a stop request. Suspension immediately revokes access; actual
container shutdown follows asynchronously while the API and Docker are available.
API shutdown does not stop containers, and restarting the API resumes supervision.
Temporary PostgreSQL supervision failures pause workspace operations while the API
reacquires exclusive ownership and verifies environment state automatically. A manual
API restart is not required for that recovery. Interrupted turns are not replayed, and
uncertain work retains its explicit recovery requirements. This does not guarantee
task completion across host outages or process failures.

Factory imposes no limit on the number of started environments, CPU, memory or process
counts. They share available host resources; Docker Desktop and operating-system limits
still apply. Containers retain a non-root user and a read-only root
filesystem. Temporary storage and logs remain bounded; persistent named volumes have no
portable disk quota. Each user has an isolated internal network; only approved OpenAI and
Anthropic HTTPS destinations are reachable through the shared installation proxy. General
Internet, host/LAN and other-user access are blocked. There is no file editor.

Existing environments with a different image or policy are never silently replaced.
A local database reset removes database records but does not delete Docker homes.

## Terminal and personal provider accounts

The terminal opens in `~/Documents` with zsh and Oh My Zsh in the current image,
shows Neofetch when its shell starts, and attaches to a persistent tmux shell. Closing the
page, losing the connection or restarting the API detaches the display without stopping
the shell. Stop environment ends its processes; saved files remain. One view owns input
at a time, and another view must explicitly take over. Reconnection never replays unsent
keystrokes. A slow display is detached while the shell continues to consume output.
Keyboard escape, screen-reader mode and mobile modifier keys are available.

The header's terminal icon opens or folds the panel without ending its shell. A draggable,
keyboard-operable separator changes its width; there is no separate width control.
Opening the panel attaches automatically, and transport failures offer contextual
reconnection. Narrow screens show chat or terminal while preserving their state.
The terminal can maximize across the application and restore the previous layout without
reattaching. The chats sidebar can collapse independently. Factory rechecks session access
in the background when returning to the page; a successful same-session check leaves
the chat and terminal in place. Revocation or expiry removes private content immediately.
The server checks input authorization and periodically closes passive revoked streams.

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

A member can connect Claude Code by running `claude auth login --claudeai` in the
private terminal and completing Anthropic's sign-in flow. Factory checks the CLI's
account status and does not receive the sign-in credentials. Settings can add either
provider after the first connection or disconnect one while retaining the other. Claude
logout, like Codex logout, is blocked while a Factory run is active or uncertain.

A missing or stopped proxy makes Codex status/login unavailable. The local terminal
and local account cancellation/logout remain independent of proxy availability, subject
to the active-run guard. Members can also deliberately run either CLI in the terminal.

## Provider conversations

After at least one provider connection, a member can create and select conversations,
choose from models reported by the connected providers, send messages, view streamed output, answer questions and request
cancellation. Completed messages and run states are retained for that user; reading
history does not replay a message or start tools. The conversation list loads older
pages on request. Each conversation shows a bounded recent history and indicates when older runs, messages or requests are omitted. A submitted
question answer can remain pending or uncertain until its receipt is confirmed; it is
not automatically resent. The last successful model list stays in browser memory for the
current session and is refreshed when stale after five minutes; account and session
changes clear it. Concurrent discovery requests share one provider operation. Temporary
model conflicts recover in the background with progressively slower retries, retaining
the current model selection. A prolonged conflict without a loaded list shows a small
status message and continues checking while the workspace is available and visible.
Models and conversations have no permanent refresh buttons; other failures offer
contextual retry. A listed model is not a guarantee that
the account can use it. Provider failures and interrupted or uncertain work remain visible.

Enter sends a message, Shift+Enter inserts a line, and IME composition does not send.
Streaming follows the newest output only while the reader is at the bottom; Jump to latest
appears when needed. User messages use quiet bubbles and agent output remains unboxed.
Assistant replies render Markdown during streaming and after completion, including
links, lists, tables and code blocks. Raw HTML cannot execute and embedded images do
not load remote resources automatically. User messages remain plain text.

Both providers run with full access inside the member's container: commands and file changes do
not ask for per-action approval, and the agent can read, change or delete any files
accessible to that container user, including saved provider credentials. Docker's
user isolation and network controls still apply, but do not protect a member's own home
from that member's agent. Treat repository content and external instructions accordingly.
Multiple accounts per provider are not yet available. When switching providers, the
visible saved conversation messages are sent as context to the selected provider. A
switch is refused if that history is truncated or too large to transfer safely. Tool
activity and answers to interactive questions are not included in the transfer.

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

The server rechecks authorization for protected operations. Initial admission requires a
verified session. Routine checks keep an already verified, unexpired workspace visible,
preserving the selected chat, draft, pane layout and scroll position without restarting
provider setup or resending input. Transient network failure/offline state keeps the
current chat visible with a connection notice until the existing fixed expiry; it does
not grant new access or enable offline execution. Expiry, authorization refusal,
invalidation or a different session discard that private UI state immediately. Sign-out
and account changes invalidate other open tabs. Private query data and provider credentials
are not persisted in localStorage, sessionStorage or IndexedDB. If sign-out cannot be
confirmed, private content stays hidden and the user can explicitly retry it.

## Installation and updates

The built frontend is installable in supported browsers. Its offline fallback is a public
page explaining that a connection is needed; installation does not restore private work
from storage while offline. An already open tab can retain its current chat in memory
until session expiry. Service-worker updates are checked on focus, reconnection and
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
