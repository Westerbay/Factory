# Product

## Current capabilities

Factory offers a private member space and an administrator backoffice. Sign-in uses
GitHub only; there are no passwords, email invitations or additional agent connections
in this increment. A successful GitHub identity check does not itself grant membership.

An operator configures the first administrator with the local CLI, selecting a GitHub
username and confirming the resolved profile. The stable provider ID is stored in the
database; web sign-in never creates an administrator. Setup refuses to replace an
existing administrator.
Another verified account becomes pending without a member session. The administrator
can activate that account, after which the member must begin a new sign-in. Handles and
display names identify people visually but never determine authorization.

The member home offers Start environment, Stop environment and Refresh for the user's
own private local environment, with observed status and observation time. Repository
selection, cloning, terminal, agent execution, history and skills are not available there.
GitHub sign-in does not request repository permissions or save a provider access token.

## Private local environments

Creating or activating an account creates no Docker resources. The first explicit start
creates one environment with a private home volume. Files under `/home/factory`, including
the working directory `/home/factory/Documents`, survive stop/start, sign-out and
suspension. There is no volume deletion or reset action. A missing previously recorded
volume produces an error rather than an empty replacement.

Start and stop requests are asynchronous. Running and Stopped reflect successful runtime
inspection, with the last observation shown; reading or refreshing status never starts
an environment. Users cannot target another user's environment. The administrator's
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
still apply. Containers retain no network access, a non-root user and a read-only root
filesystem. Temporary storage and logs remain bounded; persistent named volumes have no
portable disk quota. This increment supplies no terminal or file editor.

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
and an agent subscription, with a terminal and global skills. Providers should remain
replaceable. Repository authorization, Codex connection and execution are
separate increments, not hidden actions of sign-in.

The retained TypeScript execution implementation defines behavior to preserve during
porting: explicit task start and continuation, provider-observed parent/child agents,
independently answerable questions, explicit one-action permissions, cancellation with
confirmed cleanup, and durable history. Reading history never resumes tools. Silence,
timeouts or prose never grant approval. Only confirmed provider success completes a run;
a process exit or interrupt acknowledgement alone cannot prove descendant cleanup.

Those mechanisms currently reserve one global active root and do not implement web-user
ownership. They are reference code, not an available web capability. Existing history
must remain intact and unassigned until an explicit ownership migration is designed.
Live Codex execution, delegation and cleanup remain separate acceptance work.
