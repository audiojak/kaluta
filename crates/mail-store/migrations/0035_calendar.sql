-- Offering times (spec §14.9, ADR 0018): whether the account's Google
-- sign-in grants calendar access, recorded from the token's scopes when
-- the user connects the calendar (one row, or none), and the tentative
-- holds Kaluta made, so the user can release them.
CREATE TABLE calendar_access (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  -- calendar.freebusy: free slots can be offered.
  free_busy      INTEGER NOT NULL,
  -- calendar.events.owned: holds can be made.
  holds          INTEGER NOT NULL,
  -- The calendars whose busy times count: ["primary"].
  calendars_json TEXT NOT NULL DEFAULT '["primary"]',
  connected_at   INTEGER NOT NULL
);

CREATE TABLE calendar_holds (
  id          INTEGER PRIMARY KEY,
  -- The event's id on the calendar.
  event_id    TEXT NOT NULL,
  title       TEXT NOT NULL,
  start_at    INTEGER NOT NULL,
  end_at      INTEGER NOT NULL,
  -- The conversation the times were offered in, if any.
  thread_id   TEXT,
  created_at  INTEGER NOT NULL,
  released_at INTEGER
);
CREATE INDEX calendar_holds_active ON calendar_holds (start_at) WHERE released_at IS NULL;
