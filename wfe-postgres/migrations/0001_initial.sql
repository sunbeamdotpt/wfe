-- WFE PostgreSQL initial schema
-- Creates the wfc schema and all tables/indexes for workflow persistence.

CREATE SCHEMA IF NOT EXISTS wfc;

CREATE TABLE IF NOT EXISTS wfc.workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    root_workflow_id TEXT,
    definition_id TEXT NOT NULL,
    version INT NOT NULL,
    description TEXT,
    reference TEXT,
    status TEXT NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    next_execution BIGINT,
    create_time TIMESTAMPTZ NOT NULL,
    complete_time TIMESTAMPTZ,
    last_heartbeat_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS wfc.definition_sequences (
    definition_id TEXT PRIMARY KEY,
    next_num BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS wfc.execution_pointers (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES wfc.workflows(id),
    step_id INT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    status TEXT NOT NULL,
    sleep_until TIMESTAMPTZ,
    persistence_data JSONB,
    start_time TIMESTAMPTZ,
    end_time TIMESTAMPTZ,
    event_name TEXT,
    event_key TEXT,
    event_published BOOLEAN DEFAULT FALSE,
    event_data JSONB,
    step_name TEXT,
    retry_count INT DEFAULT 0,
    children JSONB DEFAULT '[]',
    context_item JSONB,
    predecessor_id TEXT,
    outcome JSONB,
    scope JSONB DEFAULT '[]',
    extension_attributes JSONB DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS wfc.events (
    id TEXT PRIMARY KEY,
    event_name TEXT NOT NULL,
    event_key TEXT NOT NULL,
    event_data JSONB NOT NULL DEFAULT 'null',
    event_time TIMESTAMPTZ NOT NULL,
    is_processed BOOLEAN DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS wfc.event_subscriptions (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    step_id INT NOT NULL,
    execution_pointer_id TEXT NOT NULL,
    event_name TEXT NOT NULL,
    event_key TEXT NOT NULL,
    subscribe_as_of TIMESTAMPTZ NOT NULL,
    subscription_data JSONB,
    external_token TEXT,
    external_worker_id TEXT,
    external_token_expiry TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS wfc.execution_errors (
    id SERIAL PRIMARY KEY,
    error_time TIMESTAMPTZ NOT NULL,
    workflow_id TEXT NOT NULL,
    execution_pointer_id TEXT NOT NULL,
    message TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS wfc.scheduled_commands (
    id SERIAL PRIMARY KEY,
    command_name TEXT NOT NULL,
    data TEXT NOT NULL,
    execute_time BIGINT NOT NULL,
    UNIQUE(command_name, data)
);

CREATE INDEX IF NOT EXISTS idx_workflows_next_execution ON wfc.workflows (next_execution);
CREATE INDEX IF NOT EXISTS idx_workflows_status ON wfc.workflows (status);
CREATE INDEX IF NOT EXISTS idx_events_name_key ON wfc.events (event_name, event_key);
CREATE INDEX IF NOT EXISTS idx_events_is_processed ON wfc.events (is_processed);
CREATE INDEX IF NOT EXISTS idx_events_event_time ON wfc.events (event_time);
CREATE INDEX IF NOT EXISTS idx_subscriptions_name_key ON wfc.event_subscriptions (event_name, event_key);
CREATE INDEX IF NOT EXISTS idx_subscriptions_workflow ON wfc.event_subscriptions (workflow_id);
CREATE INDEX IF NOT EXISTS idx_scheduled_commands_execute_time ON wfc.scheduled_commands (execute_time);
