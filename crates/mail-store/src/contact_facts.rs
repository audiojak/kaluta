//! Contacts and contact facts (spec §14.16, ADR 0026): a person above one
//! or more addresses, what is known about them (facts, open topics,
//! commitments and tags) with the quotes each rests on, the account's tag
//! list, and the log of what has been read to learn it. Every change is
//! recorded with its rows before and after, for exact undo (ADR 0006); an
//! item's id is never reused. Per account: none of this is in the global
//! facts store.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, StoreResult};

/// A label is refused when longer, not cut.
pub const MAX_LABEL: usize = 60;
/// A value is refused when longer, not cut.
pub const MAX_VALUE: usize = 160;
/// A tag's name is at most this long.
pub const MAX_TAG_NAME: usize = 30;

pub const KINDS: [&str; 4] = ["fact", "topic", "commitment", "tag"];
pub const CLASSES: [&str; 11] = [
    "identity",
    "role",
    "contact_detail",
    "preference",
    "personal",
    "topic",
    "money",
    "routing",
    "agreement",
    "deadline",
    "other",
];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersonRow {
    pub id: i64,
    /// The name the user gave, if any.
    pub name: Option<String>,
    pub created_at: Millis,
}

/// One address of a contact, as `contacts` has it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AddressRow {
    pub email: String,
    pub name: Option<String>,
    pub sent_count: i64,
    pub received_count: i64,
    pub last_seen: Millis,
    pub person_id: Option<i64>,
    /// When the user joined it to a person who already had an address.
    pub person_joined_at: Option<Millis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactFactRow {
    pub id: i64,
    pub person_id: i64,
    /// `fact`, `topic`, `commitment` or `tag`.
    pub kind: String,
    /// Empty for a tag.
    pub label: String,
    /// For a tag, the tag's id.
    pub value: String,
    pub class: String,
    /// Commitments: `you` or `them`.
    pub owed_by: Option<String>,
    pub due: Option<Millis>,
    /// Topics and commitments: `open` or `closed`.
    pub state: Option<String>,
    /// `free`, `ask` or `never`.
    pub use_: String,
    pub as_of: Option<Millis>,
    /// `you` or `learned`.
    pub source: String,
    /// `low` or `high`.
    pub risk: String,
    pub risk_reason: Option<String>,
    /// `watching`, `proposed`, `accepted` or `rejected`.
    pub status: String,
    /// A held change to this other item (the user's or an accepted one).
    pub changes_id: Option<i64>,
    pub seen: bool,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactEvidence {
    pub message_id: String,
    pub quote: String,
    /// `you`, `them` or `other`, decided by code.
    pub said_by: String,
    pub message_date: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    /// `relationship` or `role`.
    pub tag_set: String,
    pub heading: String,
    pub vcard_type: Option<String>,
    pub builtin: bool,
    pub position: i64,
    pub created_at: Millis,
}

/// What a change touched. Undo puts back exactly these.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub facts: Vec<(ContactFactRow, Vec<ContactEvidence>)>,
    #[serde(default)]
    pub tags: Vec<TagRow>,
    #[serde(default)]
    pub people: Vec<PersonRow>,
    /// Which person each address belongs to (and since when).
    #[serde(default)]
    pub addresses: Vec<(String, Option<i64>, Option<Millis>)>,
    /// The read log of a forgotten contact.
    #[serde(default)]
    pub reads: Vec<(String, i64, Millis)>,
}

/// The keys a change names, from both of its sides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Touched {
    pub fact_ids: Vec<i64>,
    pub tag_ids: Vec<String>,
    pub person_ids: Vec<i64>,
    pub emails: Vec<String>,
    /// People whose read log the change removes or puts back.
    pub read_person_ids: Vec<i64>,
}

// --- People and addresses ---------------------------------------------

const ADDRESS_COLUMNS: &str = "email, name, sent_count, received_count, last_seen, person_id, person_joined_at";

fn address(r: &Row<'_>) -> rusqlite::Result<AddressRow> {
    Ok(AddressRow {
        email: r.get(0)?,
        name: r.get(1)?,
        sent_count: r.get(2)?,
        received_count: r.get(3)?,
        last_seen: r.get(4)?,
        person_id: r.get(5)?,
        person_joined_at: r.get(6)?,
    })
}

pub fn person(conn: &Connection, id: i64) -> StoreResult<Option<PersonRow>> {
    Ok(conn
        .prepare_cached("SELECT id, name, created_at FROM people WHERE id = ?1")?
        .query_row([id], |r| Ok(PersonRow { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)? }))
        .optional()?)
}

pub fn address_row(conn: &Connection, email: &str) -> StoreResult<Option<AddressRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {ADDRESS_COLUMNS} FROM contacts WHERE email = ?1"))?
        .query_row([email], address)
        .optional()?)
}

/// The person an address belongs to, if one has been made.
pub fn person_for_email(conn: &Connection, email: &str) -> StoreResult<Option<i64>> {
    Ok(address_row(conn, email)?.and_then(|a| a.person_id))
}

/// A person's addresses, the one written to most first.
pub fn addresses(conn: &Connection, person_id: i64) -> StoreResult<Vec<AddressRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {ADDRESS_COLUMNS} FROM contacts WHERE person_id = ?1 ORDER BY sent_count DESC, last_seen DESC, email"
        ))?
        .query_map([person_id], address)?
        .collect::<Result<_, _>>()?)
}

/// The person for an address, made now if there is none (and the address
/// too, when mail has not recorded it).
pub fn ensure_person(tx: &Transaction<'_>, email: &str, now: Millis) -> StoreResult<i64> {
    let email = email.trim();
    if email.is_empty() {
        return Err(StoreError::Invalid("a contact needs an address".into()));
    }
    if let Some(id) = person_for_email(tx, email)? {
        return Ok(id);
    }
    tx.prepare_cached("INSERT OR IGNORE INTO contacts (email) VALUES (?1)")?.execute([email])?;
    tx.prepare_cached("INSERT INTO people (created_at) VALUES (?1)")?.execute([now])?;
    let id = tx.last_insert_rowid();
    tx.prepare_cached("UPDATE contacts SET person_id = ?2, person_joined_at = NULL WHERE email = ?1")?
        .execute(params![email, id])?;
    Ok(id)
}

/// Whether an address is one that never answers (the test Waiting uses).
pub fn is_automated_address(email: &str) -> bool {
    let email = email.to_lowercase();
    crate::waiting::AUTOMATED.iter().any(|a| email.contains(a))
}

/// Whether an address is a person's: not a no-reply or automated sender,
/// and not a mailing list (an address the user never wrote to whose mail
/// carries a `List-Id`). Only people are contacts.
pub fn is_person(conn: &Connection, email: &str) -> StoreResult<bool> {
    if email.trim().is_empty() || !email.contains('@') || is_automated_address(email) {
        return Ok(false);
    }
    let written_to = address_row(conn, email)?.is_some_and(|a| a.sent_count > 0);
    if written_to {
        return Ok(true);
    }
    let list: bool = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE from_email = ?1 COLLATE NOCASE AND list_id IS NOT NULL)",
        )?
        .query_row([email], |r| r.get(0))?;
    Ok(!list)
}

/// Move a person's read log to another (joining two contacts).
pub fn move_reads(tx: &Transaction<'_>, from: i64, into: i64) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT OR IGNORE INTO contact_reads (message_id, person_id, read_at)
         SELECT message_id, ?2, read_at FROM contact_reads WHERE person_id = ?1",
    )?
    .execute(params![from, into])?;
    tx.prepare_cached("DELETE FROM contact_reads WHERE person_id = ?1")?.execute([from])?;
    Ok(())
}

pub fn delete_reads(tx: &Transaction<'_>, person_id: i64) -> StoreResult<()> {
    tx.prepare_cached("DELETE FROM contact_reads WHERE person_id = ?1")?.execute([person_id])?;
    Ok(())
}

pub fn put_person(tx: &Transaction<'_>, p: &PersonRow) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO people (id, name, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, created_at = excluded.created_at",
    )?
    .execute(params![p.id, p.name, p.created_at])?;
    Ok(())
}

pub fn delete_person(tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
    tx.prepare_cached("UPDATE contacts SET person_id = NULL, person_joined_at = NULL WHERE person_id = ?1")?
        .execute([id])?;
    tx.prepare_cached("DELETE FROM people WHERE id = ?1")?.execute([id])?;
    Ok(())
}

/// Say which person an address belongs to.
pub fn set_address_person(
    tx: &Transaction<'_>,
    email: &str,
    person_id: Option<i64>,
    joined_at: Option<Millis>,
) -> StoreResult<()> {
    tx.prepare_cached("UPDATE contacts SET person_id = ?2, person_joined_at = ?3 WHERE email = ?1")?
        .execute(params![email, person_id, joined_at])?;
    Ok(())
}

// --- Items -------------------------------------------------------------

const FACT_COLUMNS: &str = "id, person_id, kind, label, value, class, owed_by, due, state, use, as_of, source, risk,
     risk_reason, status, changes_id, seen, created_at, updated_at";

fn fact(r: &Row<'_>) -> rusqlite::Result<ContactFactRow> {
    Ok(ContactFactRow {
        id: r.get(0)?,
        person_id: r.get(1)?,
        kind: r.get(2)?,
        label: r.get(3)?,
        value: r.get(4)?,
        class: r.get(5)?,
        owed_by: r.get(6)?,
        due: r.get(7)?,
        state: r.get(8)?,
        use_: r.get(9)?,
        as_of: r.get(10)?,
        source: r.get(11)?,
        risk: r.get(12)?,
        risk_reason: r.get(13)?,
        status: r.get(14)?,
        changes_id: r.get(15)?,
        seen: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
    })
}

/// Why a row may not be stored, if it may not: lengths are refused, not
/// cut, so an item cannot carry a passage of someone's mail.
pub fn check(f: &ContactFactRow) -> StoreResult<()> {
    let bad = |m: String| Err(StoreError::Invalid(m));
    if !KINDS.contains(&f.kind.as_str()) {
        return bad(format!("unknown kind {:?}", f.kind));
    }
    if !CLASSES.contains(&f.class.as_str()) {
        return bad(format!("unknown class {:?}", f.class));
    }
    if f.label.chars().count() > MAX_LABEL {
        return bad(format!("a label is at most {MAX_LABEL} characters"));
    }
    if f.value.chars().count() > MAX_VALUE {
        return bad(format!("a value is at most {MAX_VALUE} characters"));
    }
    if f.value.trim().is_empty() || (f.kind != "tag" && f.label.trim().is_empty()) {
        return bad("an item needs a label and a value".into());
    }
    Ok(())
}

/// A person's items in any of `statuses`, newest first.
pub fn list_for_person(conn: &Connection, person_id: i64, statuses: &[&str]) -> StoreResult<Vec<ContactFactRow>> {
    let all: Vec<ContactFactRow> = conn
        .prepare_cached(&format!(
            "SELECT {FACT_COLUMNS} FROM contact_facts WHERE person_id = ?1
             ORDER BY COALESCE(as_of, updated_at) DESC, id DESC"
        ))?
        .query_map([person_id], fact)?
        .collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|f| statuses.contains(&f.status.as_str())).collect())
}

/// Every item in any of `statuses`.
pub fn list(conn: &Connection, statuses: &[&str]) -> StoreResult<Vec<ContactFactRow>> {
    let all: Vec<ContactFactRow> = conn
        .prepare_cached(&format!("SELECT {FACT_COLUMNS} FROM contact_facts ORDER BY person_id, id"))?
        .query_map([], fact)?
        .collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|f| statuses.contains(&f.status.as_str())).collect())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<ContactFactRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {FACT_COLUMNS} FROM contact_facts WHERE id = ?1"))?
        .query_row([id], fact)
        .optional()?)
}

/// The live item with this label for a person and kind (a tag: with this
/// tag id as `label`), if there is one.
pub fn find(conn: &Connection, person_id: i64, kind: &str, label: &str) -> StoreResult<Option<ContactFactRow>> {
    let sql = if kind == "tag" {
        format!(
            "SELECT {FACT_COLUMNS} FROM contact_facts
             WHERE person_id = ?1 AND kind = ?2 AND value = ?3 AND status <> 'rejected'"
        )
    } else {
        format!(
            "SELECT {FACT_COLUMNS} FROM contact_facts
             WHERE person_id = ?1 AND kind = ?2 AND label = ?3 COLLATE NOCASE AND status <> 'rejected'
               AND changes_id IS NULL"
        )
    };
    Ok(conn.prepare_cached(&sql)?.query_row(params![person_id, kind, label], fact).optional()?)
}

/// Insert an item (a new id when `id` is 0, else that id, for undo).
pub fn insert(tx: &Transaction<'_>, f: &ContactFactRow) -> StoreResult<i64> {
    check(f)?;
    let id = (f.id != 0).then_some(f.id);
    tx.prepare_cached(
        "INSERT INTO contact_facts (id, person_id, kind, label, value, class, owed_by, due, state, use, as_of, source,
           risk, risk_reason, status, changes_id, seen, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
    )?
    .execute(params![
        id,
        f.person_id,
        f.kind,
        f.label,
        f.value,
        f.class,
        f.owed_by,
        f.due,
        f.state,
        f.use_,
        f.as_of,
        f.source,
        f.risk,
        f.risk_reason,
        f.status,
        f.changes_id,
        f.seen,
        f.created_at,
        f.updated_at
    ])?;
    Ok(tx.last_insert_rowid())
}

/// Write an item back; `false` when it is not there.
pub fn write(tx: &Transaction<'_>, f: &ContactFactRow) -> StoreResult<bool> {
    check(f)?;
    Ok(tx
        .prepare_cached(
            "UPDATE contact_facts SET person_id = ?2, kind = ?3, label = ?4, value = ?5, class = ?6, owed_by = ?7,
               due = ?8, state = ?9, use = ?10, as_of = ?11, source = ?12, risk = ?13, risk_reason = ?14,
               status = ?15, changes_id = ?16, seen = ?17, created_at = ?18, updated_at = ?19 WHERE id = ?1",
        )?
        .execute(params![
            f.id,
            f.person_id,
            f.kind,
            f.label,
            f.value,
            f.class,
            f.owed_by,
            f.due,
            f.state,
            f.use_,
            f.as_of,
            f.source,
            f.risk,
            f.risk_reason,
            f.status,
            f.changes_id,
            f.seen,
            f.created_at,
            f.updated_at
        ])?
        > 0)
}

pub fn delete(tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
    tx.prepare_cached("DELETE FROM contact_facts WHERE id = ?1")?.execute([id])?;
    Ok(())
}

/// Mark a person's accepted items as seen by the user (no longer *New*).
/// Not a change the user undoes.
pub fn mark_seen(tx: &Transaction<'_>, person_id: i64) -> StoreResult<usize> {
    Ok(tx
        .prepare_cached("UPDATE contact_facts SET seen = 1 WHERE person_id = ?1 AND seen = 0 AND status = 'accepted'")?
        .execute([person_id])?)
}

pub fn evidence(conn: &Connection, fact_id: i64) -> StoreResult<Vec<ContactEvidence>> {
    Ok(conn
        .prepare_cached(
            "SELECT message_id, quote, said_by, message_date FROM contact_fact_evidence
             WHERE fact_id = ?1 ORDER BY message_date, added_at, message_id",
        )?
        .query_map([fact_id], |r| {
            Ok(ContactEvidence { message_id: r.get(0)?, quote: r.get(1)?, said_by: r.get(2)?, message_date: r.get(3)? })
        })?
        .collect::<Result<_, _>>()?)
}

pub fn add_evidence(tx: &Transaction<'_>, fact_id: i64, quotes: &[ContactEvidence], now: Millis) -> StoreResult<()> {
    let mut add = tx.prepare_cached(
        "INSERT OR IGNORE INTO contact_fact_evidence (fact_id, message_id, quote, said_by, message_date, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for q in quotes {
        add.execute(params![fact_id, q.message_id, q.quote, q.said_by, q.message_date, now])?;
    }
    Ok(())
}

/// People with something to show: an accepted or proposed item. With how
/// many are held and the newest item's time, newest first.
pub fn people_with_items(conn: &Connection) -> StoreResult<Vec<(i64, i64, Millis)>> {
    Ok(conn
        .prepare_cached(
            "SELECT person_id, SUM(status = 'proposed'), MAX(updated_at) FROM contact_facts
             WHERE status IN ('accepted', 'proposed') GROUP BY person_id ORDER BY 3 DESC, 1",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

/// How many items wait for the user's review.
pub fn held_count(conn: &Connection) -> StoreResult<i64> {
    Ok(conn
        .prepare_cached("SELECT COUNT(*) FROM contact_facts WHERE status = 'proposed'")?
        .query_row([], |r| r.get(0))?)
}

// --- Tags --------------------------------------------------------------

const TAG_COLUMNS: &str = "id, name, tag_set, heading, vcard_type, builtin, position, created_at";

fn tag(r: &Row<'_>) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: r.get(0)?,
        name: r.get(1)?,
        tag_set: r.get(2)?,
        heading: r.get(3)?,
        vcard_type: r.get(4)?,
        builtin: r.get(5)?,
        position: r.get(6)?,
        created_at: r.get(7)?,
    })
}

/// The account's tag list: relationships, then roles, in order.
pub fn tags(conn: &Connection) -> StoreResult<Vec<TagRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {TAG_COLUMNS} FROM contact_tags ORDER BY tag_set, position, name COLLATE NOCASE, id"
        ))?
        .query_map([], tag)?
        .collect::<Result<_, _>>()?)
}

pub fn get_tag(conn: &Connection, id: &str) -> StoreResult<Option<TagRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {TAG_COLUMNS} FROM contact_tags WHERE id = ?1"))?
        .query_row([id], tag)
        .optional()?)
}

/// Tags with this name (any case; hyphens stand for spaces) or this id.
/// A name in both sets gives both.
pub fn tags_named(conn: &Connection, name: &str) -> StoreResult<Vec<TagRow>> {
    let wanted = tag_key(name);
    Ok(tags(conn)?
        .into_iter()
        .filter(|t| t.id.eq_ignore_ascii_case(name.trim()) || tag_key(&t.name) == wanted)
        .collect())
}

/// A tag name as it is compared: lower case, with hyphens and runs of
/// spaces as one space.
pub fn tag_key(name: &str) -> String {
    name.replace('-', " ").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn put_tag(tx: &Transaction<'_>, t: &TagRow) -> StoreResult<()> {
    let length = t.name.trim().chars().count();
    if length == 0 || length > MAX_TAG_NAME {
        return Err(StoreError::Invalid(format!("a tag's name is 1 to {MAX_TAG_NAME} characters")));
    }
    if !["relationship", "role"].contains(&t.tag_set.as_str()) {
        return Err(StoreError::Invalid(format!("unknown tag set {:?}", t.tag_set)));
    }
    tx.prepare_cached(
        "INSERT INTO contact_tags (id, name, tag_set, heading, vcard_type, builtin, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, tag_set = excluded.tag_set,
           heading = excluded.heading, vcard_type = excluded.vcard_type, builtin = excluded.builtin,
           position = excluded.position, created_at = excluded.created_at",
    )?
    .execute(params![
        t.id,
        t.name.trim(),
        t.tag_set,
        t.heading,
        t.vcard_type,
        t.builtin,
        t.position,
        t.created_at
    ])?;
    Ok(())
}

pub fn delete_tag(tx: &Transaction<'_>, id: &str) -> StoreResult<()> {
    tx.prepare_cached("DELETE FROM contact_tags WHERE id = ?1")?.execute([id])?;
    Ok(())
}

/// The items that put this tag on a contact, whatever their status.
pub fn items_with_tag(conn: &Connection, tag_id: &str) -> StoreResult<Vec<i64>> {
    Ok(conn
        .prepare_cached("SELECT id FROM contact_facts WHERE kind = 'tag' AND value = ?1 ORDER BY id")?
        .query_map([tag_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// How many contacts carry each accepted tag.
pub fn tag_counts(conn: &Connection) -> StoreResult<Vec<(String, i64)>> {
    Ok(conn
        .prepare_cached(
            "SELECT value, COUNT(DISTINCT person_id) FROM contact_facts
             WHERE kind = 'tag' AND status = 'accepted' GROUP BY value",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

/// The addresses (lower case) of everyone with any of these accepted tags.
pub fn emails_with_tags(conn: &Connection, tag_ids: &[String]) -> StoreResult<Vec<String>> {
    let mut out = Vec::new();
    let mut find = conn.prepare_cached(
        "SELECT DISTINCT lower(c.email) FROM contact_facts f JOIN contacts c ON c.person_id = f.person_id
         WHERE f.kind = 'tag' AND f.status = 'accepted' AND f.value = ?1",
    )?;
    for id in tag_ids {
        for email in find.query_map([id], |r| r.get::<_, String>(0))? {
            out.push(email?);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

// --- Undo ----------------------------------------------------------------

/// The rows a change names, as they are now.
pub fn snapshot(conn: &Connection, touched: &Touched) -> StoreResult<Snapshot> {
    let mut out = Snapshot::default();
    for id in &touched.fact_ids {
        if let Some(f) = get(conn, *id)? {
            out.facts.push((f, evidence(conn, *id)?));
        }
    }
    for id in &touched.tag_ids {
        out.tags.extend(get_tag(conn, id)?);
    }
    for id in &touched.person_ids {
        out.people.extend(person(conn, *id)?);
    }
    for email in &touched.emails {
        if let Some(a) = address_row(conn, email)? {
            out.addresses.push((a.email, a.person_id, a.person_joined_at));
        }
    }
    for id in &touched.read_person_ids {
        out.reads.extend(reads_of(conn, *id)?);
    }
    Ok(out)
}

/// Put the named rows back as `to` has them: rows it lacks are removed.
pub fn restore(tx: &Transaction<'_>, touched: &Touched, to: &Snapshot, now: Millis) -> StoreResult<()> {
    // People first (items refer to them), and removed last.
    for p in &to.people {
        put_person(tx, p)?;
    }
    for id in &touched.tag_ids {
        match to.tags.iter().find(|t| &t.id == id) {
            Some(t) => put_tag(tx, t)?,
            None => delete_tag(tx, id)?,
        }
    }
    // Remove before writing, so a put-back item never meets the one that
    // took its label.
    for id in &touched.fact_ids {
        if !to.facts.iter().any(|(f, _)| f.id == *id) {
            delete(tx, *id)?;
        }
    }
    // Rows that stay are set aside first (a rejected row holds no label or
    // tag), so two that swap a label or a person never meet half-way.
    for id in &touched.fact_ids {
        tx.prepare_cached("UPDATE contact_facts SET status = 'rejected' WHERE id = ?1")?.execute([id])?;
    }
    for id in &touched.fact_ids {
        if let Some((f, quotes)) = to.facts.iter().find(|(f, _)| f.id == *id) {
            if !write(tx, f)? {
                insert(tx, f)?;
            }
            tx.prepare_cached("DELETE FROM contact_fact_evidence WHERE fact_id = ?1")?.execute([id])?;
            add_evidence(tx, *id, quotes, now)?;
        }
    }
    for email in &touched.emails {
        let (person_id, joined) = to
            .addresses
            .iter()
            .find(|(e, _, _)| e.eq_ignore_ascii_case(email))
            .map(|(_, p, j)| (*p, *j))
            .unwrap_or((None, None));
        set_address_person(tx, email, person_id, joined)?;
    }
    for id in &touched.read_person_ids {
        tx.prepare_cached("DELETE FROM contact_reads WHERE person_id = ?1")?.execute([id])?;
    }
    let mut put =
        tx.prepare_cached("INSERT OR IGNORE INTO contact_reads (message_id, person_id, read_at) VALUES (?1, ?2, ?3)")?;
    for (message, person_id, at) in &to.reads {
        put.execute(params![message, person_id, at])?;
    }
    for id in &touched.person_ids {
        if !to.people.iter().any(|p| p.id == *id) {
            delete_person(tx, *id)?;
        }
    }
    Ok(())
}

pub fn record_change(
    tx: &Transaction<'_>,
    reason: &str,
    before: &Snapshot,
    after: &Snapshot,
    now: Millis,
) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO contact_fact_changes (reason, before_json, after_json, created_at) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![reason, serde_json::to_string(before)?, serde_json::to_string(after)?, now])?;
    let id = tx.last_insert_rowid();
    // Far more than the undo stack keeps.
    tx.prepare_cached("DELETE FROM contact_fact_changes WHERE id <= ?1")?.execute([id - 200])?;
    Ok(id)
}

pub fn get_change(conn: &Connection, id: i64) -> StoreResult<Option<(String, Snapshot, Snapshot)>> {
    let row: Option<(String, String, String)> = conn
        .prepare_cached("SELECT reason, before_json, after_json FROM contact_fact_changes WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match row {
        Some((reason, before, after)) => Some((reason, serde_json::from_str(&before)?, serde_json::from_str(&after)?)),
        None => None,
    })
}

// --- The read log --------------------------------------------------------

/// The daily job's reads are logged for everyone in the conversation.
pub const READ_BY_JOB: i64 = 0;

/// Log messages as read: by the job ([`READ_BY_JOB`]) or a person's deep dive.
pub fn mark_read(tx: &Transaction<'_>, message_ids: &[String], person_id: i64, now: Millis) -> StoreResult<()> {
    let mut put =
        tx.prepare_cached("INSERT OR IGNORE INTO contact_reads (message_id, person_id, read_at) VALUES (?1, ?2, ?3)")?;
    for id in message_ids {
        put.execute(params![id, person_id, now])?;
    }
    Ok(())
}

/// Whether the message has been read for this person: by their deep dive,
/// or by the job, which reads for everyone.
pub fn was_read(conn: &Connection, message_id: &str, person_id: i64) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM contact_reads WHERE message_id = ?1 AND person_id IN (?2, 0))")?
        .query_row(params![message_id, person_id], |r| r.get(0))?)
}

fn reads_of(conn: &Connection, person_id: i64) -> StoreResult<Vec<(String, i64, Millis)>> {
    Ok(conn
        .prepare_cached(
            "SELECT message_id, person_id, read_at FROM contact_reads WHERE person_id = ?1 ORDER BY message_id",
        )?
        .query_map([person_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

/// How many conversations the job read between two times (the settings
/// line: today and yesterday).
pub fn job_reads_between(conn: &Connection, from: Millis, to: Millis) -> StoreResult<i64> {
    Ok(conn
        .prepare_cached(
            "SELECT COUNT(DISTINCT m.thread_id) FROM contact_reads r JOIN messages m ON m.gmail_id = r.message_id
             WHERE r.person_id = 0 AND r.read_at >= ?1 AND r.read_at < ?2",
        )?
        .query_row(params![from, to], |r| r.get(0))?)
}

// --- Settings ------------------------------------------------------------

pub fn setting(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM contact_settings WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(tx: &Transaction<'_>, key: &str, value: Option<&str>) -> StoreResult<()> {
    match value {
        Some(v) => tx
            .prepare_cached("INSERT OR REPLACE INTO contact_settings (key, value) VALUES (?1, ?2)")?
            .execute(params![key, v])?,
        None => tx.prepare_cached("DELETE FROM contact_settings WHERE key = ?1")?.execute([key])?,
    };
    Ok(())
}

// --- Deep dives ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunRow {
    pub id: i64,
    pub person_id: i64,
    pub batch_id: Option<i64>,
    /// `queued`, `running`, `paused`, `done`, `stopped` or `failed`.
    pub state: String,
    pub max_messages: i64,
    pub total_messages: i64,
    pub done_messages: i64,
    pub turns: i64,
    pub merged: bool,
    pub error: Option<String>,
    pub started_at: Millis,
    pub updated_at: Millis,
    pub finished_at: Option<Millis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BatchRow {
    pub id: i64,
    pub days: i64,
    /// `running`, `paused`, `done` or `stopped`.
    pub state: String,
    pub total_people: i64,
    pub done_people: i64,
    pub started_at: Millis,
    pub updated_at: Millis,
    pub finished_at: Option<Millis>,
}

const RUN_COLUMNS: &str = "id, person_id, batch_id, state, max_messages, total_messages, done_messages, turns, merged,
     error, started_at, updated_at, finished_at";

fn run(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        person_id: r.get(1)?,
        batch_id: r.get(2)?,
        state: r.get(3)?,
        max_messages: r.get(4)?,
        total_messages: r.get(5)?,
        done_messages: r.get(6)?,
        turns: r.get(7)?,
        merged: r.get(8)?,
        error: r.get(9)?,
        started_at: r.get(10)?,
        updated_at: r.get(11)?,
        finished_at: r.get(12)?,
    })
}

pub fn insert_run(tx: &Transaction<'_>, r: &RunRow) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO contact_runs (person_id, batch_id, state, max_messages, total_messages, done_messages, turns,
           merged, error, started_at, updated_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?
    .execute(params![
        r.person_id,
        r.batch_id,
        r.state,
        r.max_messages,
        r.total_messages,
        r.done_messages,
        r.turns,
        r.merged,
        r.error,
        r.started_at,
        r.updated_at,
        r.finished_at
    ])?;
    Ok(tx.last_insert_rowid())
}

pub fn write_run(tx: &Transaction<'_>, r: &RunRow) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE contact_runs SET state = ?2, max_messages = ?3, total_messages = ?4, done_messages = ?5, turns = ?6,
           merged = ?7, error = ?8, updated_at = ?9, finished_at = ?10 WHERE id = ?1",
    )?
    .execute(params![
        r.id,
        r.state,
        r.max_messages,
        r.total_messages,
        r.done_messages,
        r.turns,
        r.merged,
        r.error,
        r.updated_at,
        r.finished_at
    ])?;
    Ok(())
}

pub fn get_run(conn: &Connection, id: i64) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM contact_runs WHERE id = ?1"))?
        .query_row([id], run)
        .optional()?)
}

/// A person's newest deep dive.
pub fn latest_run(conn: &Connection, person_id: i64) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLUMNS} FROM contact_runs WHERE person_id = ?1 ORDER BY id DESC LIMIT 1"
        ))?
        .query_row([person_id], run)
        .optional()?)
}

/// Runs in any of `states`, oldest first.
pub fn runs_in(conn: &Connection, states: &[&str]) -> StoreResult<Vec<RunRow>> {
    let all: Vec<RunRow> = conn
        .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM contact_runs ORDER BY id"))?
        .query_map([], run)?
        .collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|r| states.contains(&r.state.as_str())).collect())
}

pub fn runs_of_batch(conn: &Connection, batch_id: i64) -> StoreResult<Vec<RunRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM contact_runs WHERE batch_id = ?1 ORDER BY id"))?
        .query_map([batch_id], run)?
        .collect::<Result<_, _>>()?)
}

fn batch(r: &Row<'_>) -> rusqlite::Result<BatchRow> {
    Ok(BatchRow {
        id: r.get(0)?,
        days: r.get(1)?,
        state: r.get(2)?,
        total_people: r.get(3)?,
        done_people: r.get(4)?,
        started_at: r.get(5)?,
        updated_at: r.get(6)?,
        finished_at: r.get(7)?,
    })
}

const BATCH_COLUMNS: &str = "id, days, state, total_people, done_people, started_at, updated_at, finished_at";

pub fn insert_batch(tx: &Transaction<'_>, b: &BatchRow) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO contact_run_batches (days, state, total_people, done_people, started_at, updated_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?
    .execute(params![
        b.days,
        b.state,
        b.total_people,
        b.done_people,
        b.started_at,
        b.updated_at,
        b.finished_at
    ])?;
    Ok(tx.last_insert_rowid())
}

pub fn write_batch(tx: &Transaction<'_>, b: &BatchRow) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE contact_run_batches SET state = ?2, total_people = ?3, done_people = ?4, updated_at = ?5,
           finished_at = ?6 WHERE id = ?1",
    )?
    .execute(params![b.id, b.state, b.total_people, b.done_people, b.updated_at, b.finished_at])?;
    Ok(())
}

pub fn get_batch(conn: &Connection, id: i64) -> StoreResult<Option<BatchRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {BATCH_COLUMNS} FROM contact_run_batches WHERE id = ?1"))?
        .query_row([id], batch)
        .optional()?)
}

/// The newest deep dive for many, if there has been one.
pub fn latest_batch(conn: &Connection) -> StoreResult<Option<BatchRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {BATCH_COLUMNS} FROM contact_run_batches ORDER BY id DESC LIMIT 1"))?
        .query_row([], batch)
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str) -> crate::Db {
        let dir = std::env::temp_dir().join(format!("kaluta-contact-facts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::Db::open(&dir.join("mail.sqlite")).unwrap()
    }

    fn item(person_id: i64, kind: &str, label: &str, value: &str) -> ContactFactRow {
        ContactFactRow {
            person_id,
            kind: kind.into(),
            label: label.into(),
            value: value.into(),
            class: "other".into(),
            use_: "free".into(),
            source: "learned".into(),
            risk: "low".into(),
            status: "accepted".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn the_migration_applies_to_an_existing_store_and_seeds_the_tags() {
        let dir = std::env::temp_dir().join(format!("kaluta-contact-facts-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mail.sqlite");
        {
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            for (i, sql) in crate::db::migrations().iter().enumerate().take(45) {
                tx.execute_batch(sql).unwrap();
                tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1)).unwrap();
            }
            tx.execute("INSERT INTO contacts (email, name, sent_count) VALUES ('reese@example.com', 'Reese', 3)", [])
                .unwrap();
            tx.commit().unwrap();
        }
        let db = crate::Db::open(&path).unwrap();
        let (tags, old) = db.read_blocking(|c| Ok((tags(c)?, address_row(c, "REESE@example.com")?))).unwrap();
        assert_eq!(old.unwrap().person_id, None, "an address with no person is its own contact");
        assert_eq!(tags.iter().filter(|t| t.tag_set == "relationship").count(), 61);
        assert_eq!(tags.iter().filter(|t| t.tag_set == "role").count(), 77);
        assert!(tags.iter().all(|t| t.builtin && t.name.chars().count() <= MAX_TAG_NAME));
        let vcard = |id: &str| tags.iter().find(|t| t.id == id).unwrap().vcard_type.clone();
        assert_eq!(vcard("rel.colleague").as_deref(), Some("co-worker"));
        assert_eq!(vcard("rel.peer").as_deref(), Some("colleague"));
        assert_eq!(vcard("rel.customer"), None);
        assert_eq!(vcard("role.vp"), None);
    }

    #[test]
    fn a_person_is_made_on_first_need_and_items_keep_one_per_label() {
        let db = open("items");
        db.write_blocking(|tx| {
            let reese = ensure_person(tx, "reese@example.com", 10)?;
            assert_eq!(ensure_person(tx, "Reese@Example.com", 11)?, reese);
            let id = insert(tx, &item(reese, "topic", "Renewal pricing for 2027", "Waiting on their numbers"))?;
            // A second live item with the label is refused; a rejected one is not in the way.
            assert!(insert(tx, &item(reese, "topic", "renewal pricing for 2027", "Again")).is_err());
            let mut rejected = item(reese, "topic", "Renewal pricing for 2027", "Not this");
            rejected.status = "rejected".into();
            insert(tx, &rejected)?;
            // The same label under another kind is its own item.
            insert(tx, &item(reese, "fact", "Renewal pricing for 2027", "They decide in March"))?;
            assert_eq!(find(tx, reese, "topic", "RENEWAL pricing for 2027")?.unwrap().id, id);
            assert_eq!(list_for_person(tx, reese, &["accepted"])?.len(), 2);
            assert_eq!(addresses(tx, reese)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn over_long_labels_and_values_are_refused_not_cut() {
        let db = open("lengths");
        db.write_blocking(|tx| {
            let p = ensure_person(tx, "a@example.com", 1)?;
            assert!(insert(tx, &item(p, "fact", &"l".repeat(61), "v")).is_err());
            assert!(insert(tx, &item(p, "fact", "l", &"v".repeat(161))).is_err());
            let id = insert(tx, &item(p, "fact", &"é".repeat(60), &"ü".repeat(160)))?;
            let mut longer = get(tx, id)?.unwrap();
            longer.value.push('x');
            assert!(write(tx, &longer).is_err());
            assert_eq!(get(tx, id)?.unwrap().value.chars().count(), 160);
            assert!(insert(tx, &item(p, "fact", "", "v")).is_err());
            assert!(insert(tx, &item(p, "note", "l", "v")).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn evidence_watching_and_undo() {
        let db = open("evidence");
        db.write_blocking(|tx| {
            let p = ensure_person(tx, "a@example.com", 1)?;
            let mut watching = item(p, "fact", "Role", "CTO at Acme");
            watching.status = "watching".into();
            let id = insert(tx, &watching)?;
            let quote = |m: &str, day: Millis| ContactEvidence {
                message_id: m.into(),
                quote: "I'm the CTO".into(),
                said_by: "them".into(),
                message_date: day,
            };
            add_evidence(tx, id, &[quote("m1", 100)], 5)?;
            add_evidence(tx, id, &[quote("m1", 100), quote("m2", 200)], 6)?;
            assert_eq!(evidence(tx, id)?.len(), 2, "a message is one piece of evidence");
            assert!(list_for_person(tx, p, &["accepted", "proposed"])?.is_empty(), "watching is not shown");
            assert!(people_with_items(tx)?.is_empty());

            // Accept it, recorded; then undo puts the row and its quotes back.
            let touched = Touched { fact_ids: vec![id], ..Default::default() };
            let before = snapshot(tx, &touched)?;
            let mut accepted = get(tx, id)?.unwrap();
            accepted.status = "accepted".into();
            write(tx, &accepted)?;
            tx.execute("DELETE FROM contact_fact_evidence WHERE message_id = 'm2'", [])?;
            let after = snapshot(tx, &touched)?;
            let change = record_change(tx, "accept", &before, &after, 7)?;
            assert_eq!(people_with_items(tx)?.len(), 1);
            let (reason, was, _) = get_change(tx, change)?.unwrap();
            assert_eq!(reason, "accept");
            restore(tx, &touched, &was, 8)?;
            assert_eq!(get(tx, id)?.unwrap().status, "watching");
            assert_eq!(evidence(tx, id)?.len(), 2);
            // Undoing an insert removes the row.
            restore(tx, &touched, &Snapshot::default(), 9)?;
            assert!(get(tx, id)?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_are_one_each_and_found_by_name() {
        let db = open("tags");
        db.write_blocking(|tx| {
            let p = ensure_person(tx, "a@example.com", 1)?;
            insert(tx, &item(p, "tag", "", "rel.customer"))?;
            assert!(insert(tx, &item(p, "tag", "", "rel.customer")).is_err(), "a contact has each tag once");
            insert(tx, &item(p, "tag", "", "role.vp"))?;
            let mut held = item(p, "tag", "", "rel.vendor");
            held.status = "proposed".into();
            insert(tx, &held)?;
            assert_eq!(tags_named(tx, "services-client")?[0].id, "rel.services-client");
            assert_eq!(tags_named(tx, "SERVICES  client")?[0].id, "rel.services-client");
            assert_eq!(tags_named(tx, "rel.customer")?[0].name, "Customer");
            assert_eq!(tags_named(tx, "board member")?.len(), 2, "a name in both sets gives both");
            assert!(tags_named(tx, "wizard")?.is_empty());
            assert_eq!(tag_counts(tx)?.len(), 2, "held tags are not counted");
            assert_eq!(emails_with_tags(tx, &["rel.customer".into()])?, vec!["a@example.com".to_string()]);
            assert!(emails_with_tags(tx, &["rel.vendor".into()])?.is_empty(), "a held tag finds nobody");
            // The user's own tag; a second with the name in the set is refused.
            let mine = TagRow {
                id: "user.1".into(),
                name: "Design partner".into(),
                tag_set: "relationship".into(),
                heading: "Other".into(),
                created_at: 1,
                ..Default::default()
            };
            put_tag(tx, &mine)?;
            assert!(
                put_tag(tx, &TagRow { id: "user.2".into(), name: "design PARTNER".into(), ..mine.clone() }).is_err()
            );
            assert!(put_tag(tx, &TagRow { id: "user.3".into(), name: "x".repeat(31), ..mine.clone() }).is_err());
            // A rename is one row.
            put_tag(tx, &TagRow { name: "Launch partner".into(), ..mine })?;
            assert_eq!(get_tag(tx, "user.1")?.unwrap().name, "Launch partner");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_read_log_is_per_message_and_contact() {
        let db = open("reads");
        db.write_blocking(|tx| {
            let a = ensure_person(tx, "a@example.com", 1)?;
            let b = ensure_person(tx, "b@example.com", 1)?;
            mark_read(tx, &["m1".into()], a, 5)?;
            mark_read(tx, &["m1".into()], a, 6)?;
            assert!(was_read(tx, "m1", a)?);
            assert!(!was_read(tx, "m1", b)?, "a deep dive reads for its own person");
            mark_read(tx, &["m2".into()], READ_BY_JOB, 7)?;
            assert!(was_read(tx, "m2", a)? && was_read(tx, "m2", b)?, "the job reads for everyone");
            // Forgetting a contact takes their read log, and undo puts it back.
            let touched = Touched { read_person_ids: vec![a], ..Default::default() };
            let before = snapshot(tx, &touched)?;
            restore(tx, &touched, &Snapshot::default(), 8)?;
            assert!(!was_read(tx, "m1", a)?);
            restore(tx, &touched, &before, 9)?;
            assert!(was_read(tx, "m1", a)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn settings_and_runs_are_kept() {
        let db = open("runs");
        db.write_blocking(|tx| {
            assert_eq!(setting(tx, "learn")?, None);
            set_setting(tx, "learn", Some("on"))?;
            assert_eq!(setting(tx, "learn")?.as_deref(), Some("on"));
            set_setting(tx, "learn", None)?;
            assert_eq!(setting(tx, "learn")?, None);
            let p = ensure_person(tx, "a@example.com", 1)?;
            let batch_id = insert_batch(
                tx,
                &BatchRow { days: 90, state: "running".into(), total_people: 1, ..Default::default() },
            )?;
            let id = insert_run(
                tx,
                &RunRow {
                    person_id: p,
                    batch_id: Some(batch_id),
                    state: "queued".into(),
                    max_messages: 2000,
                    ..Default::default()
                },
            )?;
            let mut r = get_run(tx, id)?.unwrap();
            r.state = "running".into();
            r.done_messages = 20;
            r.turns = 1;
            write_run(tx, &r)?;
            assert_eq!(runs_in(tx, &["running"])?, vec![r.clone()]);
            assert_eq!(latest_run(tx, p)?.unwrap().turns, 1);
            assert_eq!(runs_of_batch(tx, batch_id)?.len(), 1);
            assert_eq!(latest_batch(tx)?.unwrap().days, 90);
            Ok(())
        })
        .unwrap();
    }
}
