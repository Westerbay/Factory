CREATE TABLE "agents" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"parentId" text,
	CONSTRAINT "agents_runId_id_pk" PRIMARY KEY("runId","id")
);
--> statement-breakpoint
CREATE TABLE "events" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"sequence" integer NOT NULL,
	"event" jsonb NOT NULL,
	CONSTRAINT "events_runId_id_pk" PRIMARY KEY("runId","id"),
	CONSTRAINT "events_identity" CHECK (("events"."event"->>'id' = "events"."id" and "events"."event"->>'runId' = "events"."runId" and "events"."sequence" > 0) is true)
);
--> statement-breakpoint
CREATE TABLE "messages" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "messages_runId_id_pk" PRIMARY KEY("runId","id")
);
--> statement-breakpoint
CREATE TABLE "permissions" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "permissions_runId_id_pk" PRIMARY KEY("runId","id")
);
--> statement-breakpoint
CREATE TABLE "questions" (
	"runId" text NOT NULL,
	"id" text NOT NULL,
	"agentId" text NOT NULL,
	CONSTRAINT "questions_runId_id_pk" PRIMARY KEY("runId","id")
);
--> statement-breakpoint
CREATE TABLE "runs" (
	"id" text PRIMARY KEY NOT NULL,
	"rootAgentId" text NOT NULL,
	"version" integer NOT NULL,
	"status" text NOT NULL,
	"createdAt" text NOT NULL,
	"projection" jsonb NOT NULL,
	CONSTRAINT "runs_version_positive" CHECK ("runs"."version" > 0),
	CONSTRAINT "runs_projection_identity" CHECK (("runs"."projection"->>'id' = "runs"."id" and ("runs"."projection"->>'version')::integer = "runs"."version" and "runs"."projection"->>'status' = "runs"."status" and "runs"."projection"->>'rootAgentId' = "runs"."rootAgentId") is true),
	CONSTRAINT "runs_status_known" CHECK ("runs"."status" in ('created','running','waiting-for-user','cancelling','completed','failed','interrupted'))
);
--> statement-breakpoint
ALTER TABLE "agents" ADD CONSTRAINT "agents_runId_runs_id_fk" FOREIGN KEY ("runId") REFERENCES "public"."runs"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "agents" ADD CONSTRAINT "agents_parent_owned" FOREIGN KEY ("runId","parentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "events" ADD CONSTRAINT "events_runId_runs_id_fk" FOREIGN KEY ("runId") REFERENCES "public"."runs"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "messages" ADD CONSTRAINT "messages_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "permissions" ADD CONSTRAINT "permissions_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "questions" ADD CONSTRAINT "questions_agent_owned" FOREIGN KEY ("runId","agentId") REFERENCES "public"."agents"("runId","id") ON DELETE no action ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "events_sequence_unique" ON "events" USING btree ("runId","sequence");--> statement-breakpoint
CREATE UNIQUE INDEX "runs_one_active_root" ON "runs" USING btree ((true)) WHERE "runs"."status" in ('created','running','waiting-for-user','cancelling');
--> statement-breakpoint
-- Drizzle 0.45 cannot represent a deferrable foreign key. Keep this manual constraint:
-- a projection is inserted before its root agent in the same transaction.
ALTER TABLE "runs" ADD CONSTRAINT "runs_root_owned" FOREIGN KEY ("id", "rootAgentId") REFERENCES "agents"("runId", "id") DEFERRABLE INITIALLY DEFERRED;
