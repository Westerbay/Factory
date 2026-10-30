# Design

The web interface uses semantic HTML, system fonts and small replaceable styles. Template
branding and page layouts are not requirements. Shared presentation belongs in components;
session and API behavior belongs in shared libraries, not copied between features.

The public sign-in screen has one GitHub action and readable pending, suspended, denied,
unavailable and invalid-login states. The authenticated header identifies the user, links
to their workspace, exposes user administration only to administrators and offers sign-out.
The workspace communicates which capabilities are still unavailable without fake data or
inactive controls that imply working agents or projects.

Private routes show content only after a current session check. Loading, offline and
unavailable states replace stale private content. Errors offer an explicit recovery
action; an unconfirmed sign-out keeps private content hidden and offers retry. Status
messages use text and accessible live regions, not color alone.

The workspace shares one environment status controller across the environment card,
terminal and Codex account card. Terminal and account actions require a confirmed running
environment on the current policy. An explicit upgrade requirement disables Start and new
connections, explains that administrator action is needed, and keeps Stop available for
a running or uncertain environment. The interface never upgrades a container implicitly.

The terminal loads on explicit Open, reconnects only on request and requires an explicit
takeover when another tab owns input. Its accessible xterm area has an exit-focus action,
responsive sizing and mobile Enter, Tab and Ctrl controls. A transport interruption does
not replay queued input. Closing the view detaches from the persistent shell; Stop ends
running processes while preserving files.

Codex sign-in shows a private device code, explicit Copy action and a validated official
OpenAI link opened with `noopener noreferrer`. It reports pending, cancelled, expired,
connected, unavailable and uncertain states without claiming a displayed code is success.
Mutations move focus to a stable status element when their buttons disappear; background
polling does not steal focus. Polling runs only during sign-in on an active page. Returning
to the page reads the existing operation and never starts another sign-in automatically.
Code and account data stay in memory and disappear on session invalidation, account change
or unavailable environment. Expired codes disappear even when a request is stalled.
Disconnect explains that manually launched Codex processes may still require Stop.

The access backoffice uses readable user cards with provider, role and status. Filters
and pagination remain in the URL. Pending mutations disable only the affected row;
access is not changed optimistically. A conflict refreshes server state and asks the user
to review it. Administrators display a protected label instead of access-edit controls.

Suspension opens a native modal dialog describing session revocation. Focus begins on
Cancel; Escape cancels and returns focus to the originating button. Confirmation is
explicit. Submitting an access change moves focus to a stable result message that
remains present after the affected row disappears. Completion does not steal focus
from a subsequent interaction. Visible focus outlines, labels and accessible control
names are required.
Controls and identity text wrap at narrow widths, including a 360-pixel viewport.

The PWA update notice is non-blocking and provides an explicit update/reload action.
The offline page contains only public instructions. No installed-app state should suggest
that a cached private session remains usable without server verification.
API responses, terminal output and account data are excluded from the service-worker cache.

Keep English UI text close to its owning component until an actual localization runtime
is needed. Reuse shadcn primitives when an implemented interaction benefits from them;
do not copy an unused catalog. Future agent output must remain inert until a deliberate
rich-content policy is implemented, and questions and permissions must retain separate,
explicit actions.
