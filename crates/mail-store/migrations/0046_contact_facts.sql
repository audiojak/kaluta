-- Contacts and contact facts (spec §14.16, ADR 0026): a person above one
-- or more addresses, what Kaluta knows about them (facts, open topics,
-- commitments and tags) with where each came from, and the log of what
-- was read to learn it. Facts about the user (0017) are unchanged.
CREATE TABLE people (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  -- The name the user gave; else the page shows the addresses' names.
  name       TEXT,
  created_at INTEGER NOT NULL
);

-- An address with no person is its own contact; a person is made on
-- first need. `person_joined_at` is when the user joined this address to
-- a person who already had another (the review gate distrusts new ones).
-- No foreign key: the store clears it when a person is deleted.
ALTER TABLE contacts ADD COLUMN person_id INTEGER;
ALTER TABLE contacts ADD COLUMN person_joined_at INTEGER;
CREATE INDEX contacts_by_person ON contacts (person_id) WHERE person_id IS NOT NULL;

-- The account's tag list. `id` never changes (built-in: a fixed slug;
-- the user's: generated), so a rename is one row.
CREATE TABLE contact_tags (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 30),
  tag_set    TEXT NOT NULL CHECK (tag_set IN ('relationship', 'role')),
  heading    TEXT NOT NULL,
  -- vCard RELATED type (RFC 6350) where one says the same thing.
  vcard_type TEXT,
  builtin    INTEGER NOT NULL DEFAULT 0,
  position   INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE UNIQUE INDEX contact_tags_by_name ON contact_tags (tag_set, name COLLATE NOCASE);

CREATE TABLE contact_facts (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  person_id   INTEGER NOT NULL REFERENCES people (id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('fact', 'topic', 'commitment', 'tag')),
  -- Empty for a tag, whose value is the tag's id.
  label       TEXT NOT NULL DEFAULT '' CHECK (length(label) <= 60),
  value       TEXT NOT NULL CHECK (length(value) <= 160),
  class       TEXT NOT NULL DEFAULT 'other' CHECK (class IN ('identity', 'role', 'contact_detail', 'preference',
                'personal', 'topic', 'money', 'routing', 'agreement', 'deadline', 'other')),
  -- Commitments only.
  owed_by     TEXT CHECK (owed_by IN ('you', 'them')),
  due         INTEGER,
  -- Topics and commitments only.
  state       TEXT CHECK (state IN ('open', 'closed')),
  use         TEXT NOT NULL DEFAULT 'free' CHECK (use IN ('free', 'ask', 'never')),
  as_of       INTEGER,
  source      TEXT NOT NULL CHECK (source IN ('you', 'learned')),
  risk        TEXT NOT NULL DEFAULT 'low' CHECK (risk IN ('low', 'high')),
  risk_reason TEXT,
  -- `watching`: seen once, in no prompt and not on the page.
  status      TEXT NOT NULL CHECK (status IN ('watching', 'proposed', 'accepted', 'rejected')),
  -- A change the mail asks for to an item the user wrote or accepted,
  -- held with the item it would change.
  changes_id  INTEGER REFERENCES contact_facts (id) ON DELETE CASCADE,
  -- Whether the user has seen it on the page (accepted ones show as New).
  seen        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE INDEX contact_facts_by_person ON contact_facts (person_id, kind, status);
-- One live item per label within a contact and kind; one of each tag.
CREATE UNIQUE INDEX contact_facts_one_label ON contact_facts (person_id, kind, label COLLATE NOCASE)
  WHERE kind <> 'tag' AND status <> 'rejected' AND changes_id IS NULL;
CREATE UNIQUE INDEX contact_facts_one_tag ON contact_facts (person_id, value)
  WHERE kind = 'tag' AND status <> 'rejected';

CREATE TABLE contact_fact_evidence (
  fact_id      INTEGER NOT NULL REFERENCES contact_facts (id) ON DELETE CASCADE,
  message_id   TEXT NOT NULL,
  quote        TEXT NOT NULL,
  -- Decided by code from the message's sender, never by the agent.
  said_by      TEXT NOT NULL CHECK (said_by IN ('you', 'them', 'other')),
  -- The message's date, for two sightings on different days.
  message_date INTEGER NOT NULL DEFAULT 0,
  added_at     INTEGER NOT NULL,
  PRIMARY KEY (fact_id, message_id)
) WITHOUT ROWID;

-- Every change, for exact undo (ADR 0006).
CREATE TABLE contact_fact_changes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  reason      TEXT NOT NULL,
  before_json TEXT NOT NULL,
  after_json  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);

-- What has been read for contact facts, so nothing is read twice.
-- `person_id` 0: read by the daily job, for everyone in the conversation;
-- else by that person's deep dive.
CREATE TABLE contact_reads (
  message_id TEXT NOT NULL,
  person_id  INTEGER NOT NULL DEFAULT 0,
  read_at    INTEGER NOT NULL,
  PRIMARY KEY (message_id, person_id)
) WITHOUT ROWID;

-- A deep dive for many (Learn From Past Mail).
CREATE TABLE contact_run_batches (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  days         INTEGER NOT NULL,
  state        TEXT NOT NULL CHECK (state IN ('running', 'paused', 'done', 'stopped')),
  total_people INTEGER NOT NULL DEFAULT 0,
  done_people  INTEGER NOT NULL DEFAULT 0,
  started_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  finished_at  INTEGER
);

-- Deep dives, as guide_runs: progress that survives a quit.
CREATE TABLE contact_runs (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  person_id      INTEGER NOT NULL REFERENCES people (id) ON DELETE CASCADE,
  batch_id       INTEGER REFERENCES contact_run_batches (id) ON DELETE CASCADE,
  state          TEXT NOT NULL CHECK (state IN ('queued', 'running', 'paused', 'done', 'stopped', 'failed')),
  max_messages   INTEGER NOT NULL DEFAULT 2000,
  total_messages INTEGER NOT NULL DEFAULT 0,
  done_messages  INTEGER NOT NULL DEFAULT 0,
  turns          INTEGER NOT NULL DEFAULT 0,
  merged         INTEGER NOT NULL DEFAULT 0,
  error          TEXT,
  started_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  finished_at    INTEGER
);
CREATE INDEX contact_runs_by_state ON contact_runs (state, id);

-- Per-account switches for learning about contacts (off until the user
-- says yes): key and value, as text.
CREATE TABLE contact_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;

INSERT INTO contact_tags (id, name, tag_set, heading, vcard_type, builtin, position, created_at) VALUES
  ('rel.customer', 'Customer', 'relationship', 'They buy from me', NULL, 1, 0, 0),
  ('rel.services-client', 'Services client', 'relationship', 'They buy from me', NULL, 1, 1, 0),
  ('rel.prospect', 'Prospect', 'relationship', 'They buy from me', NULL, 1, 2, 0),
  ('rel.lead', 'Lead', 'relationship', 'They buy from me', NULL, 1, 3, 0),
  ('rel.former-customer', 'Former customer', 'relationship', 'They buy from me', NULL, 1, 4, 0),
  ('rel.reseller', 'Reseller', 'relationship', 'They buy from me', NULL, 1, 5, 0),
  ('rel.distributor', 'Distributor', 'relationship', 'They buy from me', NULL, 1, 6, 0),
  ('rel.user', 'User', 'relationship', 'They buy from me', NULL, 1, 7, 0),
  ('rel.vendor', 'Vendor', 'relationship', 'I buy from them', NULL, 1, 8, 0),
  ('rel.supplier', 'Supplier', 'relationship', 'I buy from them', NULL, 1, 9, 0),
  ('rel.contractor', 'Contractor', 'relationship', 'I buy from them', NULL, 1, 10, 0),
  ('rel.freelancer', 'Freelancer', 'relationship', 'I buy from them', NULL, 1, 11, 0),
  ('rel.agency', 'Agency', 'relationship', 'I buy from them', NULL, 1, 12, 0),
  ('rel.consultant', 'Consultant', 'relationship', 'I buy from them', NULL, 1, 13, 0),
  ('rel.professional-adviser', 'Professional adviser', 'relationship', 'I buy from them', NULL, 1, 14, 0),
  ('rel.service-provider', 'Service provider', 'relationship', 'I buy from them', NULL, 1, 15, 0),
  ('rel.landlord', 'Landlord', 'relationship', 'I buy from them', NULL, 1, 16, 0),
  ('rel.colleague', 'Colleague', 'relationship', 'Work with me', 'co-worker', 1, 17, 0),
  ('rel.my-manager', 'My manager', 'relationship', 'Work with me', NULL, 1, 18, 0),
  ('rel.direct-report', 'Direct report', 'relationship', 'Work with me', NULL, 1, 19, 0),
  ('rel.co-founder', 'Co-founder', 'relationship', 'Work with me', NULL, 1, 20, 0),
  ('rel.business-partner', 'Business partner', 'relationship', 'Work with me', NULL, 1, 21, 0),
  ('rel.collaborator', 'Collaborator', 'relationship', 'Work with me', NULL, 1, 22, 0),
  ('rel.former-colleague', 'Former colleague', 'relationship', 'Work with me', NULL, 1, 23, 0),
  ('rel.board-member', 'Board member', 'relationship', 'Work with me', NULL, 1, 24, 0),
  ('rel.employer', 'Employer', 'relationship', 'Work with me', NULL, 1, 25, 0),
  ('rel.investor', 'Investor', 'relationship', 'Money and advice', NULL, 1, 26, 0),
  ('rel.prospective-investor', 'Prospective investor', 'relationship', 'Money and advice', NULL, 1, 27, 0),
  ('rel.portfolio-company', 'Portfolio company', 'relationship', 'Money and advice', NULL, 1, 28, 0),
  ('rel.lender', 'Lender', 'relationship', 'Money and advice', NULL, 1, 29, 0),
  ('rel.advisor', 'Advisor', 'relationship', 'Money and advice', NULL, 1, 30, 0),
  ('rel.mentor', 'Mentor', 'relationship', 'Money and advice', NULL, 1, 31, 0),
  ('rel.mentee', 'Mentee', 'relationship', 'Money and advice', NULL, 1, 32, 0),
  ('rel.coach', 'Coach', 'relationship', 'Money and advice', NULL, 1, 33, 0),
  ('rel.trainer', 'Trainer', 'relationship', 'Money and advice', NULL, 1, 34, 0),
  ('rel.candidate', 'Candidate', 'relationship', 'Hiring', NULL, 1, 35, 0),
  ('rel.recruiter', 'Recruiter', 'relationship', 'Hiring', NULL, 1, 36, 0),
  ('rel.reference', 'Reference', 'relationship', 'Hiring', NULL, 1, 37, 0),
  ('rel.prospective-employer', 'Prospective employer', 'relationship', 'Hiring', NULL, 1, 38, 0),
  ('rel.competitor', 'Competitor', 'relationship', 'Field and public', NULL, 1, 39, 0),
  ('rel.peer', 'Peer', 'relationship', 'Field and public', 'colleague', 1, 40, 0),
  ('rel.press', 'Press', 'relationship', 'Field and public', NULL, 1, 41, 0),
  ('rel.analyst', 'Analyst', 'relationship', 'Field and public', NULL, 1, 42, 0),
  ('rel.regulator', 'Regulator', 'relationship', 'Field and public', NULL, 1, 43, 0),
  ('rel.auditor', 'Auditor', 'relationship', 'Field and public', NULL, 1, 44, 0),
  ('rel.government', 'Government', 'relationship', 'Field and public', NULL, 1, 45, 0),
  ('rel.event-organizer', 'Event organizer', 'relationship', 'Field and public', NULL, 1, 46, 0),
  ('rel.community-member', 'Community member', 'relationship', 'Field and public', NULL, 1, 47, 0),
  ('rel.contributor', 'Contributor', 'relationship', 'Field and public', NULL, 1, 48, 0),
  ('rel.teacher', 'Teacher', 'relationship', 'Learning', NULL, 1, 49, 0),
  ('rel.student', 'Student', 'relationship', 'Learning', NULL, 1, 50, 0),
  ('rel.classmate', 'Classmate', 'relationship', 'Learning', NULL, 1, 51, 0),
  ('rel.alumni', 'Alumni', 'relationship', 'Learning', NULL, 1, 52, 0),
  ('rel.family', 'Family', 'relationship', 'Personal', 'kin', 1, 53, 0),
  ('rel.spouse-or-partner', 'Spouse or partner', 'relationship', 'Personal', 'spouse', 1, 54, 0),
  ('rel.friend', 'Friend', 'relationship', 'Personal', 'friend', 1, 55, 0),
  ('rel.acquaintance', 'Acquaintance', 'relationship', 'Personal', 'acquaintance', 1, 56, 0),
  ('rel.neighbour', 'Neighbour', 'relationship', 'Personal', 'neighbor', 1, 57, 0),
  ('rel.tenant', 'Tenant', 'relationship', 'Personal', NULL, 1, 58, 0),
  ('rel.healthcare-provider', 'Healthcare provider', 'relationship', 'Personal', NULL, 1, 59, 0),
  ('rel.school-or-childcare', 'School or childcare', 'relationship', 'Personal', NULL, 1, 60, 0),
  ('role.founder', 'Founder', 'role', 'Level', NULL, 1, 61, 0),
  ('role.owner', 'Owner', 'role', 'Level', NULL, 1, 62, 0),
  ('role.board-member', 'Board member', 'role', 'Level', NULL, 1, 63, 0),
  ('role.chair', 'Chair', 'role', 'Level', NULL, 1, 64, 0),
  ('role.executive', 'Executive', 'role', 'Level', NULL, 1, 65, 0),
  ('role.vp', 'VP', 'role', 'Level', NULL, 1, 66, 0),
  ('role.director', 'Director', 'role', 'Level', NULL, 1, 67, 0),
  ('role.manager', 'Manager', 'role', 'Level', NULL, 1, 68, 0),
  ('role.team-lead', 'Team lead', 'role', 'Level', NULL, 1, 69, 0),
  ('role.individual-contributor', 'Individual contributor', 'role', 'Level', NULL, 1, 70, 0),
  ('role.assistant', 'Assistant', 'role', 'Level', NULL, 1, 71, 0),
  ('role.intern', 'Intern', 'role', 'Level', NULL, 1, 72, 0),
  ('role.self-employed', 'Self-employed', 'role', 'Level', NULL, 1, 73, 0),
  ('role.retired', 'Retired', 'role', 'Level', NULL, 1, 74, 0),
  ('role.ceo', 'CEO', 'role', 'Title', NULL, 1, 75, 0),
  ('role.president', 'President', 'role', 'Title', NULL, 1, 76, 0),
  ('role.managing-director', 'Managing Director', 'role', 'Title', NULL, 1, 77, 0),
  ('role.coo', 'COO', 'role', 'Title', NULL, 1, 78, 0),
  ('role.cfo', 'CFO', 'role', 'Title', NULL, 1, 79, 0),
  ('role.cto', 'CTO', 'role', 'Title', NULL, 1, 80, 0),
  ('role.cio', 'CIO', 'role', 'Title', NULL, 1, 81, 0),
  ('role.ciso', 'CISO', 'role', 'Title', NULL, 1, 82, 0),
  ('role.cpo', 'CPO', 'role', 'Title', NULL, 1, 83, 0),
  ('role.cmo', 'CMO', 'role', 'Title', NULL, 1, 84, 0),
  ('role.cro', 'CRO', 'role', 'Title', NULL, 1, 85, 0),
  ('role.general-counsel', 'General Counsel', 'role', 'Title', NULL, 1, 86, 0),
  ('role.chief-of-staff', 'Chief of Staff', 'role', 'Title', NULL, 1, 87, 0),
  ('role.general-partner', 'General Partner', 'role', 'Title', NULL, 1, 88, 0),
  ('role.principal', 'Principal', 'role', 'Title', NULL, 1, 89, 0),
  ('role.engineering', 'Engineering', 'role', 'Function', NULL, 1, 90, 0),
  ('role.technical', 'Technical', 'role', 'Function', NULL, 1, 91, 0),
  ('role.product', 'Product', 'role', 'Function', NULL, 1, 92, 0),
  ('role.design', 'Design', 'role', 'Function', NULL, 1, 93, 0),
  ('role.data', 'Data', 'role', 'Function', NULL, 1, 94, 0),
  ('role.research', 'Research', 'role', 'Function', NULL, 1, 95, 0),
  ('role.security', 'Security', 'role', 'Function', NULL, 1, 96, 0),
  ('role.it', 'IT', 'role', 'Function', NULL, 1, 97, 0),
  ('role.sales', 'Sales', 'role', 'Function', NULL, 1, 98, 0),
  ('role.business-development', 'Business Development', 'role', 'Function', NULL, 1, 99, 0),
  ('role.partnerships', 'Partnerships', 'role', 'Function', NULL, 1, 100, 0),
  ('role.marketing', 'Marketing', 'role', 'Function', NULL, 1, 101, 0),
  ('role.communications', 'Communications', 'role', 'Function', NULL, 1, 102, 0),
  ('role.customer-success', 'Customer Success', 'role', 'Function', NULL, 1, 103, 0),
  ('role.support', 'Support', 'role', 'Function', NULL, 1, 104, 0),
  ('role.finance', 'Finance', 'role', 'Function', NULL, 1, 105, 0),
  ('role.accounting', 'Accounting', 'role', 'Function', NULL, 1, 106, 0),
  ('role.legal', 'Legal', 'role', 'Function', NULL, 1, 107, 0),
  ('role.compliance', 'Compliance', 'role', 'Function', NULL, 1, 108, 0),
  ('role.people', 'People', 'role', 'Function', NULL, 1, 109, 0),
  ('role.recruiting', 'Recruiting', 'role', 'Function', NULL, 1, 110, 0),
  ('role.operations', 'Operations', 'role', 'Function', NULL, 1, 111, 0),
  ('role.procurement', 'Procurement', 'role', 'Function', NULL, 1, 112, 0),
  ('role.facilities', 'Facilities', 'role', 'Function', NULL, 1, 113, 0),
  ('role.administration', 'Administration', 'role', 'Function', NULL, 1, 114, 0),
  ('role.lawyer', 'Lawyer', 'role', 'Profession', NULL, 1, 115, 0),
  ('role.accountant', 'Accountant', 'role', 'Profession', NULL, 1, 116, 0),
  ('role.banker', 'Banker', 'role', 'Profession', NULL, 1, 117, 0),
  ('role.financial-adviser', 'Financial adviser', 'role', 'Profession', NULL, 1, 118, 0),
  ('role.insurance-agent', 'Insurance agent', 'role', 'Profession', NULL, 1, 119, 0),
  ('role.real-estate-agent', 'Real estate agent', 'role', 'Profession', NULL, 1, 120, 0),
  ('role.doctor', 'Doctor', 'role', 'Profession', NULL, 1, 121, 0),
  ('role.nurse', 'Nurse', 'role', 'Profession', NULL, 1, 122, 0),
  ('role.therapist', 'Therapist', 'role', 'Profession', NULL, 1, 123, 0),
  ('role.teacher', 'Teacher', 'role', 'Profession', NULL, 1, 124, 0),
  ('role.professor', 'Professor', 'role', 'Profession', NULL, 1, 125, 0),
  ('role.researcher', 'Researcher', 'role', 'Profession', NULL, 1, 126, 0),
  ('role.journalist', 'Journalist', 'role', 'Profession', NULL, 1, 127, 0),
  ('role.writer', 'Writer', 'role', 'Profession', NULL, 1, 128, 0),
  ('role.editor', 'Editor', 'role', 'Profession', NULL, 1, 129, 0),
  ('role.designer', 'Designer', 'role', 'Profession', NULL, 1, 130, 0),
  ('role.developer', 'Developer', 'role', 'Profession', NULL, 1, 131, 0),
  ('role.architect', 'Architect', 'role', 'Profession', NULL, 1, 132, 0),
  ('role.tradesperson', 'Tradesperson', 'role', 'Profession', NULL, 1, 133, 0),
  ('role.civil-servant', 'Civil servant', 'role', 'Profession', NULL, 1, 134, 0),
  ('role.clergy', 'Clergy', 'role', 'Profession', NULL, 1, 135, 0),
  ('role.nonprofit-staff', 'Nonprofit staff', 'role', 'Profession', NULL, 1, 136, 0),
  ('role.student', 'Student', 'role', 'Profession', NULL, 1, 137, 0);
