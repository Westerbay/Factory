# Product

## Interface language

The profile settings offer English and French. Without a saved choice, Factory uses the
first supported browser language, falling back to English. A manual choice is retained
in that browser and synchronized across its open tabs. Agent responses and terminal output remain in their original language.

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
Stop environment, a light/dark theme choice and Sign out, plus User access for administrators.
The initial theme follows the browser preference and the choice is stored on the device;
there is no separate system mode. Provider account
details, additional connections and disconnect actions belong in Settings. GitHub sign-in
requests repository access as well as identity. The interface uses Space Mono and concise
contextual status instead of an environment dashboard or permanent refresh controls.

## GitHub projects

A member authorizes repository access during GitHub sign-in. The app requests GitHub
`repo`, `write:public_key` and `user:email` permissions; the classic `repo` scope grants
broad read/write access. Once the environment is ready, Factory automatically registers its
dedicated SSH public key. Codex and the
terminal can use its private key with that member's GitHub permissions, including push;
repository rules and organization SSO still apply. The account's verified primary email
is used as the default Git author email, which is visible to readers of commits. Existing
explicit Git identity settings take precedence.

Accessible repositories can be imported into the environment's `~/Documents` using
the repository name as the folder name, such as `~/Documents/Factory`. If that name is
already occupied, import reports the conflict without overwriting it. The import picker
excludes existing projects and shows progress until the clone is ready; users may continue
in the background. Deleting a local project folder leaves its project and chats
listed, but the folder must be restored before its project sessions can run. Import
reserves the stable GitHub repository ID before cloning; concurrent requests never launch another clone. Interrupted or
uncertain imports require an observed environment stop and explicit recovery before a
retry. Factory never overwrites or adopts unrecognized folders. The sidebar groups chats
under independently expandable projects and offers a new-chat action on each project. Project chats start and
resume Codex or Claude Code in the selected repository; an unavailable folder is reported rather than
replacing it with a different directory.

A confirmed project deletion removes its local checkout and chats, including unpushed changes,
but leaves the GitHub repository untouched. Deletion is refused while an import or project
run is active or uncertain. Members must stop terminal commands first; terminal access to
project chats closes when deletion starts. An interrupted deletion can be retried from the
project row. Unrecognized or replaced folders are never deleted.

When a member opens a project, Factory checks whether its original GitHub path still
identifies the same repository. A redirect, inaccessible path or different repository
is reported for review; an unavailable check is shown as unverified, without blocking
local work. This check does not inspect the Git remote or establish that a push is safe:
GitHub may change after the check, and the member may have configured a different remote
or push URL. Factory never changes those Git settings automatically.

Disconnecting repository access locally removes the saved OAuth token but does not itself
remove the SSH key. Members can inspect or revoke SSH keys in GitHub settings. Revoking
all GitHub OAuth app permissions also removes keys created by that app; GitHub removes
keys unused for one year. Reconnection recovers a confirmed missing registered key.

## Private local environments

Creating or activating an account creates no Docker resources. On entry to the workspace,
an active member automatically initializes a missing environment or starts a stopped one.
An existing running environment is reused. Before starting a stopped environment,
Factory checks for recoverable runs and imports; a failed check or outstanding recovery keeps it
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
portable disk quota. Each user has an isolated internal network; approved OpenAI and
Anthropic HTTPS destinations and GitHub SSH on port 443 are reachable through the shared
installation proxy. General Internet, host/LAN and other-user access are blocked. There is no file editor.

Existing environments with a different image or policy are never silently replaced.
A local database reset removes database records but does not delete Docker homes.

## Terminal and personal provider accounts

A project chat’s terminal starts in that project repository; a projectless chat starts in
`~/Documents`. Each chat attaches to its own persistent zsh/tmux shell and shows Neofetch
when its shell starts. Switching chats or reconnecting preserves the shell and a directory
changed by the member. Closing the
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

A member can connect Claude Code using the CLI's official sign-in link. After authorizing
with Anthropic, the member enters the browser-issued one-time code in Factory, which
forwards it to the same CLI process without persisting it. The CLI must confirm the
account before Factory marks it connected. Settings can add either
provider after the first connection or disconnect one while retaining the other. Claude
logout, like Codex logout, is blocked while a Factory run is active or uncertain.

A missing or stopped proxy makes Codex status/login unavailable. The local terminal
and local account cancellation/logout remain independent of proxy availability, subject
to the active-run guard. Members can also deliberately run either CLI in the terminal.

## Features

Overview links to Chat and Features. A member starts a feature in a ready project
and selects a connected provider. The conversation opens with the assistant's first
message and uses a side panel for the current decision or result.

The member first approves the current product brief, then the current architecture
proposal and UML. A new product decision invalidates dependent architecture approval.
Members can explicitly reopen the affected gate to revise the work. Task and delivery
plans remain internal. Feature attempts show their role, assignment, actual model when
the provider reports it, activity and status. Resume checks that the prior execution
stopped and retains the verified work.

The integration is reviewed against one Git snapshot. Relevant specialists join the
general review. Corrections use separate writers and a fresh review when needed.
Decisive failed or missing checks and blocking findings prevent publication. Members
can inspect the result and explicitly request one draft pull request. A changed diff
must be reviewed again. An ambiguous publication is reconciled before retrying.

## Provider conversations

After at least one provider connection, a member can create and select conversations,
choose from models reported by the connected providers, see and choose an advertised reasoning effort
for models that support it, send messages, view streamed output, answer questions and request
cancellation. Completed messages and run states are retained for that user; reading
history does not replay a message or start tools. The conversation list loads older
pages on request. Each conversation shows a bounded recent history and indicates when older runs, messages or requests are omitted. A submitted
question answer can remain pending or uncertain until its receipt is confirmed; it is
not automatically resent. The last successful model list stays in browser memory for the
current session, including when changing tabs. Confirmed provider connection changes
refresh it; session changes clear it. Concurrent discovery requests share one provider operation. Temporary
provider discovery failures leave other providers' models usable and offer a retry for missing models. Temporary
model conflicts recover in the background with progressively slower retries, retaining
the current model selection. A prolonged conflict without a loaded list shows a small
status message and continues checking while the workspace is available and visible.
Models and conversations have no permanent refresh buttons; other failures offer
contextual retry. The model selector shows the resolved version when the provider reports it;
aliases and provider fallbacks can still change the actual model used. A listed model is not a guarantee that
the account can use it. Provider failures and interrupted or uncertain work remain visible.

Enter sends a message, Shift+Enter inserts a line, and IME composition does not send.
Streaming follows the newest output only while the reader is at the bottom; Jump to latest
appears when needed. User messages use quiet bubbles and agent output remains unboxed.
Assistant replies render Markdown during streaming and after completion, including
links, lists, tables and code blocks. Raw HTML cannot execute and embedded images do
not load remote resources automatically. User messages remain plain text.

In free chat, both providers run with full access inside the member's container: commands and file changes do
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

Future work includes global skills and additional repository providers. Providers should
remain replaceable.
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
