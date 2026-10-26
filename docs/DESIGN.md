# Design

The initial screen uses semantic HTML, system fonts and minimal replaceable CSS.
There is no product theme, template typography, branding, dashboard layout or animation.
Text explicitly distinguishes the running desktop shell from future agent/database work.

Use shadcn primitives when a later screen actually needs controls. No unused component
catalog is copied into this increment. `packages/ui` reserves that boundary without
shipping placeholder controls. Keep labels, keyboard access, visible focus, error states
and status text accessible. Do not rely on color alone to communicate state.

English messages live in the renderer catalog rather than component markup. Introduce
an actual localization runtime before adding locales or variable/plural messages.
Long content should wrap, and the shell should remain readable at narrow widths.
