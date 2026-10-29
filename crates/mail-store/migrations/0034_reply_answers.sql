-- Ready answers (spec §14.13, amended 2026-10-29): whether a conversation
-- needs a reply from the user, whatever the kind (a long answer is a task
-- and still needs one), and the different answers they might give, at
-- most three short labels: `options_json` ["Yes", "No"]. `answers_by`
-- says who wrote `answers_json`: 'triage' (the triage prompt's quick
-- answers) or 'writer' (the Reply answers job, one whole email per option
-- with the writing guide and facts).
ALTER TABLE triage_suggestions ADD COLUMN needs_reply INTEGER NOT NULL DEFAULT 0;
ALTER TABLE triage_suggestions ADD COLUMN options_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE triage_suggestions ADD COLUMN answers_by TEXT NOT NULL DEFAULT 'triage';
