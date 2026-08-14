CREATE INDEX computation_runs_actor_requested
    ON computation_runs(actor_kind, actor_id, requested_at);
CREATE INDEX computation_runs_actor_state
    ON computation_runs(actor_kind, actor_id, state);
