# Design

## Direction

Factory is a private developer workspace for sustained conversations with a coding
agent and a persistent terminal. The interface uses flat, nearly neutral surfaces, quiet navigation and readable messages
in light and graphite themes. The initial theme follows the browser preference; the profile
menu offers a direct light or dark choice, stored locally on the device. Chat is the primary
work surface. Operational details appear when they require a decision.

Sign-in, provider authorization, workspace, settings and administration share this
language. Avoid gradients, glows, glass, marketing copy, inactive future-provider
choices and ornamental cards. Keep English UI text close to its owning component.

The selected typography is **Space Mono**, self-hosted with its OFL license.
Use one shared font token; refit terminal metrics after font loading.
Regular text is 400 and emphasis 700, using actual font weights. Font selection is a
design decision, not an unfinished product preference picker.

## Foundations

[app.css](../apps/web/src/styles/app.css) owns the semantic tokens. Components use
these roles rather than repeating visual literals.

| Role                         | Graphite direction                                                 |
| ---------------------------- | ------------------------------------------------------------------ |
| Canvas                       | Graphite, `oklch(0.208 0.003 265)`                                 |
| Sidebar and terminal         | `oklch(0.174 0.003 265)`                                           |
| Raised controls and composer | `oklch(0.245 0.004 265)`                                           |
| Hover and selection          | `oklch(0.285 0.004 265)`                                           |
| Structural edge              | `oklch(0.32 0.005 265)`; never the only affordance                 |
| Text                         | `oklch(0.935 0.003 265)`                                           |
| Secondary text               | `oklch(0.71 0.006 265)`; retain 4.5:1 contrast                     |
| Positive / destructive       | Muted green / coral, only for meaningful state                     |
| Primary action               | Light surface, dark text                                           |
| Type                         | 12px metadata, 14px controls, 15–16px chat, 20–26px entry headings |
| Spacing                      | 4px base; 8/12px control gaps, 16/24px content gutters             |
| Shape                        | 6px controls, 8px composer/dialog, circular account avatar         |

The light theme provides corresponding neutral surfaces and contrasting text through the same semantic roles.

Keep prose within roughly 65 characters per line. Meaningful text is at least 12px.
Pointer targets are at least 24px and touch targets at least 44px. Keyboard focus is
visible and distinct from selection. Use one outline icon family; provider identity
uses an unmodified brand asset or its written name.

Scrollbars remain native and thin with a transparent track, a muted thumb and clearer
hover feedback. Shared scrollbar colors also feed xterm's theme. ANSI terminal text uses a minimum 4.5:1 contrast against its background. Keep scrolling
available to pointer, touch and keyboard; forced-color mode uses system scrollbars.

## Reusable components

Shared app presentation belongs in `apps/web/src/components/ui`; policy remains
feature-owned. Do not populate an unused catalog or introduce speculative
cross-application abstractions in `packages/ui`.

| Component           | Contract and consumers                                                                                         |
| ------------------- | -------------------------------------------------------------------------------------------------------------- |
| Button / IconButton | Native semantics, primary/secondary/ghost/danger variants, accessible labels for icon actions                  |
| Modal               | Native dialog, named heading, safe initial focus, Escape/cancel and focus restoration; shutdown and suspension |
| LoadingState        | Concise status and reduced-motion-safe indicator; session, environment and provider preparation                |
| Brand               | Terminal prompt symbol and wordmark with real home/workspace navigation                                        |
| SplitPane           | Pointer and keyboard resizing, one terminal toggle, no separate width toolbar                                  |

Conversation rows, composer, device authorization and account options stay
feature-owned. Share their real behavior rather than a universal card. Controls
cover hover, focus, pressed, disabled, pending and error. Links remain links.

The workspace navigation groups chats under independently expandable projects, with a
new-chat and delete actions on each project. The active chat uses
orange text without a selection background; keyboard focus remains visible. The project picker uses the
shared modal, a labelled search over loaded GitHub pages and explicit loading,
empty, unavailable and retry states. Import progress remains in the modal until confirmed,
with an option to continue in the background. Already imported repositories are omitted.
Show the selected project's name on its chat and terminal; switching chats detaches
the terminal display while preserving each chat's shell. Explain GitHub permissions
and commit email visibility before the explicit connection action. An interrupted
import exposes recovery rather than an indefinite progress indicator.

## Entry and environment lifecycle

1. Sign-in shows Factory and **Continue with GitHub**. Pending, suspended, denied,
   invalid and unavailable access states add only the applicable explanation and
   next action. GitHub identity alone never grants membership.
2. Verified workspace entry initializes a missing environment or starts a stopped
   one automatically. A running environment is reused. This is one entry action,
   never an effect of every poll, focus change or session recheck.
3. Show **Starting your workspace…** while starting. Never invent a percentage.
   A failure stays visible and offers contextual retry.
4. An unconnected account can choose **Codex** or **Claude Code**. Codex device
   authorization retains copy, cancel, expiry, the validated official OpenAI link
   and uncertain completion. Claude sign-in shows the CLI's validated link and accepts
   the one-time code returned by Claude. A displayed code does not mean connected.
5. An account with at least one connected provider proceeds to the workspace.

Explicit Stop suppresses automatic restart while the same app session remains open,
including route remounts and focus checks. The automatic start opportunity is
consumed even when the environment is already running, so another tab's Stop is
not immediately reversed. Browser reload/new tab is a new workspace entry; this
is not a durable preference stored in the browser.

Check recoverable runs before automatically starting a previously stopped
environment. If that check fails or recovery is needed, remain stopped with an
explicit recovery path. **Return to workspace** is a contextual resume after
shutdown/recovery, not a permanent Start control. Failed starts do not loop.

## Workspace

Desktop has a sidebar starting at 280px, flexible chat and an optional right terminal.
Drag the sidebar separator or use its keyboard controls to resize it; its width remains
when the sidebar is hidden or the layout switches to compact mode. The sidebar holds
the wordmark, New chat, history and GitHub profile at bottom left.
Conversation labels use available data; never invent names or introduce a data
migration just to match example content. Older history pages remain reachable.
The header's chats toggle collapses and restores the desktop sidebar without resetting
its state. Hidden navigation is inert and removed from keyboard navigation.

The header contains the current chat title and a terminal icon. Drag the separator
to resize; arrows resize from the keyboard and Enter collapses it. The terminal
icon restores it. Keep a
usable minimum chat width. Do not animate drag geometry. Hiding the terminal
preserves its mounted display and shell. Opening loads/attaches xterm without a
second Open terminal button. Reconnect appears after transport failure; taking
over input from another tab remains explicit.

The terminal header also offers maximize/restore. Maximizing fills the application
viewport without replacing the terminal instance or ending its attachment. The chat
and sidebar stay mounted but inert; restore recovers the previous split width and
sidebar choice. Escape restores the workspace after closing any terminal settings.

Below the split layout's usable width, the sidebar becomes a dismissible drawer
and chat/terminal show one pane at a time without losing state. Account options
remain reachable after shutdown. Screen-reader mode, keyboard escape from xterm
and mobile modifier keys remain supported, with secondary settings progressively
exposed.

User messages use quiet bubbles; agent output is unboxed. The composer holds the
message, model choice and send/stop. Enter sends, Shift+Enter inserts a line, and
IME composition never sends. Follow streaming only at the bottom; show Jump to
latest only when needed. Keep agent output inert, explicit question choices,
delivery uncertainty, intentional retries, history truncation and run recovery.
Never automatically replay an ambiguous message or answer. Full-access execution
consequences remain available without a permanent paragraph over every chat.

Completed and streaming agent replies share the same Markdown renderer: headings,
emphasis, lists, quotes, links, tables, task lists and code blocks. User input stays
plain text. Code blocks and tables scroll within the message on narrow screens.
Raw HTML is ignored, embedded images remain descriptive text, and only explicit
HTTP(S) or mail links are clickable. External links open separately with opener
isolation. Markdown never creates executable content or automatic remote requests.

Returning to the tab preserves the selected conversation, draft, pane layout and
manual scroll position. Routine session verification runs in the background without
hiding or disabling the established workspace. Pause environment/provider polling
while hidden and refresh silently on return; do not erase their last observations or
restart terminal attachments just because the tab lost focus. Never replay input.

Remove permanent Refresh models/Refresh conversation, the environment dashboard,
visible width slider, Disconnect display and redundant setup controls. Remove
obsolete components, CSS, props and exports after their last consumer is removed.
Contextual retry/recovery are safeguards, not replacement toolbars. Keep the last
successful model list in session memory across tab changes; confirmed account changes
refresh it and session changes clear private cached data. Temporary model conflicts must not
replace a loaded selection with an error banner. If discovery remains blocked without
any loaded models, use a compact status message while automatic recovery continues.
The composer shows a concise model name and the provider-reported version when available, without a redundant identifier, and a labeled
reasoning-effort selector only when the selected model advertises options. It shows and
sends a concrete level: the advertised default when available, otherwise medium when
supported, or the first supported level.

## Account and destructive actions

The profile menu contains Settings, administrator-only User access, Stop environment,
a light/dark theme choice and Sign out. Provider details, additional connections and disconnect actions belong in Settings. Disconnect
retains active/uncertain-run guards and explains its consequences when invoked.

Stop opens **Stop your environment?** The confirmation explains that running tasks
and terminal processes stop while saved files and completed chat messages remain. Focus starts
on Cancel. Only confirmation sends Stop. Pending, confirmed stopped and failure
are distinct; a request acknowledgment never proves shutdown.

Closing Factory and signing out leave the environment running. Suspension still
revokes access immediately and requests asynchronous shutdown. Administrator
suspension uses the same dialog conventions with server-confirmed updates,
conflict handling and stable focus after a row disappears.

## Motion

Share duration/easing tokens: about 120ms control feedback, 160ms menus and 180ms
modal/panel appearance, using a decelerating curve. Appearance uses opacity and a small
translation or scale. No bounce, blur, shimmer gradients, page-load
choreography, character-by-character streaming effects or animated drag width.
Sidebar collapse and terminal maximize use short transitions that preserve the mounted
content. Geometry changes refit the terminal; pointer resizing remains immediate.
Closing is as fast or faster than opening. Dismissal never leaves invisible focus
traps. Reduced motion removes travel and spinner rotation while retaining textual
status and feedback.

## Feature workspace

Keep the shared shell and one page header. A feature opens directly in conversation,
with a stage panel for the brief, UML, agents, or result. On compact screens the
discussion and stage panel are separate tabs. Use text and icons to distinguish
running, failed, uncertain and verified attempts. Gate controls describe the current
revision. Show publication next to the reviewed diff when the result is ready.

UML previews have fit, zoom and source access. Completed assistant code fences can
offer an explicit preview action. Authored HTML opens in an isolated frame. Keep the
conversation source readable and maintain focus when closing an expanded preview.

## Privacy and delivery

Initial admission and protected route changes require verification. An already verified,
unexpired session keeps its workspace visible during background checks. Transient
connection failure or offline state shows a compact status without discarding the
chat; it never extends the fixed expiry. Expiry, authorization refusal, logout, remote
invalidation or a different authenticated scope discards private UI immediately.
Abort/reconcile in-flight operations on access loss. Never persist credentials, device
codes, messages or private query data in browser storage. Unconfirmed logout keeps
content hidden and offers retry.

The PWA update notice stays non-blocking with explicit reload. Its offline page is
public. API/OAuth responses, terminal output and account data remain outside the
service-worker cache. The API does not serve frontend assets. See
[PRODUCT.md](PRODUCT.md) for behavior and [REUSE.md](REUSE.md) for ownership.
