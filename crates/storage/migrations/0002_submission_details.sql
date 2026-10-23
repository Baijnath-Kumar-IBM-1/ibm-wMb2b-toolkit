-- Submission details recorded by the submission engine: the rendered EDI
-- payload (no secrets), a short excerpt of the channel response, and the
-- time the post took. Nullable because rows written before this migration
-- (and generated-only records) have none of them.

ALTER TABLE submissions ADD COLUMN payload TEXT;
ALTER TABLE submissions ADD COLUMN response_excerpt TEXT;
ALTER TABLE submissions ADD COLUMN duration_ms INTEGER;
