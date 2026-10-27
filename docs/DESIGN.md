# Design

The operator interface uses semantic HTML, system fonts and small replaceable CSS.
There is no inherited template theme, fixed product branding or animation. Runtime
readiness, workspace/task controls and persisted execution history precede the selected
execution's agent relationships, output, questions, permissions and continuation.

Use shadcn primitives when a real interaction benefits from them; no unused catalog is
copied. Keep visible labels, keyboard access, native focus behavior, focus outlines and
inline errors. Status uses explicit text rather than color alone. Native close dialogs
default to keeping the window open.

Question forms are scoped to exact question and agent identities. One pending response
never disables unrelated forms. Permission controls explicitly say allow once or deny.
No default answer is selected. Resolved, submitting and invalidated states stay visible.
History selection only reads; execution requires a separate explicit action. Previous/next
page controls keep older records reachable with bounded list memory, while the selected
execution remains visible. A failed stop can be explicitly retried after the pending
request ends; it is never retried automatically.

Agent output is rendered as inert, wrapping plain text inside bounded scroll regions.
Live fragments are temporary and reconciled against completed messages. No HTML, rich
Markdown, graph engine or workflow editor is enabled. Long IDs, paths and prose wrap; the
two-column operator layout becomes one column on narrow windows.

English messages live in the renderer catalog. Introduce an actual localization runtime
before adding locales or variable/plural messages. Styling can evolve independently of
the domain, provider and persistence boundaries.
