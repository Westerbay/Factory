# Product

## Intended outcome

An operator will select a local workspace and skills, submit tasks, observe agents and
native delegation, answer questions emitted by skills, stop work and revisit history.
Skills define their questions and workflow gates; Factory will transport and display them.
A missing response will never be interpreted as approval.

## Current increment

Opening Factory shows a minimal desktop foundation screen. It explicitly states that
no agent is running and no database has been opened. There are no simulated execution
controls, invented running agents or sample history.

## Following steps

The approved sequence adds local runtime/desktop communication, PostgreSQL persistence,
the Codex adapter, the operator interface and a bounded real-agent acceptance journey.
Those are not implemented by the foundation step. Cross-provider orchestration,
remote hosting, skill authoring and an inherited template visual identity are excluded.
