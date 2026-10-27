---
name: factory-acceptance
description: Bounded acceptance scenario for Factory, using only the generated harmless workspace.
---

# Factory acceptance

Use only the selected generated workspace. Do not access other workspaces, personal
files, credentials, network services or global configuration. Do not write files,
execute repository commands, commit, install packages or request broader permissions.

1. Ask exactly one explicit question: "Which label should I use: amber or blue?"
   Prefer the provider's structured user-input tool when available. Wait for the human
   answer before reading or delegating. If that tool is unavailable, ask in the final
   conversation message and end the turn; wait for an explicit conversation continuation.
   Never invent an answer or claim conversational text was a structured request.
2. After the answer, create exactly one native child, if the provider exposes native
   delegation. Give it this bounded task: read only `harmless.txt` in this workspace,
   report the two listed objects and their total count, and finish. The child must not
   delegate, write, access other files or use network services. Do not start another child.
3. Wait for that child's result and ensure the child is finished/closed using the native
   lifecycle tools available. Summarize the result with the human's chosen label.
   If delegation is unavailable or fails, report that specific limitation and stop;
   do not simulate a child or substitute your own read for delegation evidence.

If stopped, do not resume work, answer stale requests or start a replacement child.
Report only observed provider outcomes; instruction text alone proves no capability.
