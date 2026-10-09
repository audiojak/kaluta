//! Following the writing guide (spec §14.9): the accepted guide rendered
//! as instructions for one message (its recipients' audiences, its type),
//! or in general for a session that has no message yet, and the checks an
//! AI draft's own text is put through. Rules and facts are always
//! included, with their scope written out; guidelines only where their
//! scope matches.

use serde::{Deserialize, Serialize};

use crate::audience::AudienceGroups;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Always holds (where its scope says).
    Rule,
    /// Followed unless the message calls for something else.
    Guideline,
    /// A fact entry from before Facts had their own store (spec §14.11).
    Fact,
}

/// Where an entry applies; empty everywhere means always.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    /// Audience group names.
    pub groups: Vec<String>,
    /// Addresses or `@domain`s.
    pub people: Vec<String>,
    /// `new`, `reply`, `forward`.
    pub message_types: Vec<String>,
    /// Language names or codes.
    pub languages: Vec<String>,
}

/// What a check tests on an AI draft's own text (spec §14.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// The draft must not contain the phrase (any case, whole words).
    BannedPhrase,
    /// The draft must contain it (a sign-off, required wording).
    RequiredPhrase,
    /// The draft's own text is at most this many words.
    MaxWords,
}

/// A test run on AI drafts without an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub kind: CheckKind,
    pub value: String,
}

/// One accepted entry of the guide. Never carries evidence: quotes come
/// from sent mail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    /// The guide category (`A1`, `B6`, …).
    pub category: String,
    pub kind: Kind,
    pub statement: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub check: Option<Check>,
}

/// The message being drafted, as far as it is known.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub recipients: Vec<String>,
    /// `new`, `reply` or `forward`.
    pub message_type: Option<String>,
    /// Audiences chosen for the draft instead of the recipients' own.
    pub audiences: Option<Vec<String>>,
}

impl Target {
    /// The audiences the message is written for: those chosen, or the
    /// recipients' confirmed ones.
    pub fn audiences_in(&self, groups: &AudienceGroups) -> Vec<String> {
        self.audiences.clone().unwrap_or_else(|| groups.audiences_of(&self.recipients))
    }
}

/// What drafting follows, for one message or in general.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rendered {
    /// Instructions for the agent; empty when there is no guide yet.
    pub text: String,
    /// The guide's version it was rendered from.
    pub version: i64,
    /// The recipients' confirmed audiences ("Customers").
    pub audiences: Vec<String>,
}

/// A check an AI draft failed (spec §14.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckFailure {
    pub entry_id: i64,
    pub statement: String,
    /// What is wrong, in a line: "Uses “circle back”, which your rules ban".
    pub message: String,
}

/// Every `<` becomes `‹`, so no text closes or fakes a block.
pub fn fenced(s: &str) -> String {
    s.replace('<', "‹")
}

/// Whether an entry's scope matches the message. People and audiences
/// narrow it; a scope naming what the message is not keeps it out.
/// Languages are not matched: a draft's language is not known ahead.
pub fn applies(scope: &Scope, target: &Target, audiences: &[String]) -> bool {
    applies_in(scope, target, audiences, &AudienceGroups::default())
}

/// [`applies`], with the scope's people matched as `groups` match their
/// members: hashed in a published snapshot, plain in the app.
fn applies_in(scope: &Scope, target: &Target, audiences: &[String], groups: &AudienceGroups) -> bool {
    let people = scope.people.is_empty() || target.recipients.iter().any(|r| groups.matches(r, &scope.people));
    let groups =
        scope.groups.is_empty() || scope.groups.iter().any(|g| audiences.iter().any(|a| a.eq_ignore_ascii_case(g)));
    let types = scope.message_types.is_empty()
        || target.message_type.as_ref().is_some_and(|t| scope.message_types.iter().any(|s| s == t));
    people && groups && types
}

/// A narrower scope first, so it reads as winning: people, then
/// audiences, then message types, then everyone.
fn narrowness(scope: &Scope) -> u8 {
    if !scope.people.is_empty() {
        0
    } else if !scope.groups.is_empty() {
        1
    } else if !scope.message_types.is_empty() || !scope.languages.is_empty() {
        2
    } else {
        3
    }
}

/// A scope in words: "for Customers; in reply".
pub fn scope_text(s: &Scope) -> String {
    let mut parts = Vec::new();
    if !s.groups.is_empty() {
        parts.push(format!("for {}", s.groups.join(", ")));
    }
    if !s.people.is_empty() {
        parts.push(format!("to {}", s.people.join(", ")));
    }
    if !s.message_types.is_empty() {
        parts.push(format!("in {}", s.message_types.join(", ")));
    }
    if !s.languages.is_empty() {
        // The draft's language is not known ahead: the agent applies these
        // when it writes in one of them.
        parts.push(format!("when writing in {}", s.languages.join(" or ")));
    }
    parts.join("; ")
}

/// Render the guide. `target` None: for a whole session (every guideline,
/// each with its scope); `examples`: the user's own messages to imitate
/// (never published). Returns the text and the audiences it was written
/// for.
pub fn render(
    entries: &[Entry],
    groups: &AudienceGroups,
    target: Option<&Target>,
    examples: &[String],
    version: i64,
) -> (String, Vec<String>) {
    // F3 entries left over from before Facts (spec §14.11) are not used:
    // facts render from their own store, by their use.
    let accepted: Vec<&Entry> = entries.iter().filter(|e| e.category != "F3").collect();
    if accepted.is_empty() {
        return (String::new(), vec![]);
    }
    let audiences = match target {
        Some(t) => t.audiences_in(groups),
        None => vec![],
    };
    // In a published snapshot the people an entry is for are hashes: such
    // an entry is shown only for a message to one of them, and its scope
    // names the recipients it matched, which the agent gave.
    let hashed = groups.is_hashed();
    let line = |e: &Entry| {
        let scope = match target {
            Some(t) if hashed && !e.scope.people.is_empty() => {
                let people = t
                    .recipients
                    .iter()
                    .filter(|r| groups.matches(r, &e.scope.people))
                    .map(|r| r.trim().to_lowercase())
                    .collect();
                scope_text(&Scope { people, ..e.scope.clone() })
            }
            _ => scope_text(&e.scope),
        };
        if scope.is_empty() { format!("- {}", e.statement) } else { format!("- {} ({scope})", e.statement) }
    };
    let by_kind = |kind: Kind, filter: bool| {
        let mut chosen: Vec<&&Entry> = accepted
            .iter()
            .filter(|e| e.kind == kind)
            .filter(|e| !filter || target.is_none_or(|t| applies_in(&e.scope, t, &audiences, groups)))
            .filter(|e| {
                !hashed
                    || e.scope.people.is_empty()
                    || target.is_some_and(|t| t.recipients.iter().any(|r| groups.matches(r, &e.scope.people)))
            })
            .collect();
        chosen.sort_by_key(|e| (narrowness(&e.scope), e.category.clone(), e.id));
        chosen.into_iter().map(|e| line(e)).collect::<Vec<_>>()
    };
    let rules = by_kind(Kind::Rule, false);
    let facts = by_kind(Kind::Fact, false);
    let guidelines = by_kind(Kind::Guideline, true);

    let mut out =
        format!("The user's writing guide (version {version}). Follow it in every email you draft or edit for them.\n");
    if let Some(t) = target {
        let mut about = Vec::new();
        if !audiences.is_empty() {
            about.push(format!("written for {}", audiences.join(", ")));
        }
        if let Some(kind) = &t.message_type {
            about.push(format!("a {}", if kind == "new" { "new message" } else { kind }));
        }
        if !about.is_empty() {
            out.push_str(&format!("This message: {}.\n", about.join("; ")));
        }
    }
    if !rules.is_empty() {
        out.push_str(&format!(
            "\nRules (always; a rule with a scope holds where its scope says):\n{}\n",
            rules.join("\n")
        ));
    }
    if !facts.is_empty() {
        out.push_str(&format!("\nFacts about the user you may use:\n{}\n", facts.join("\n")));
    }
    if !guidelines.is_empty() {
        out.push_str(&format!(
            "\nGuidelines ({}; follow them unless the message calls for something else):\n{}\n",
            if target.is_some() { "for this message" } else { "each where its scope says" },
            guidelines.join("\n")
        ));
    }
    let for_people_unseen =
        hashed && target.is_none_or(|t| t.recipients.is_empty()) && accepted.iter().any(|e| !e.scope.people.is_empty());
    if for_people_unseen {
        out.push_str(
            "\nSome entries are for particular people and are shown only when the message's recipients are \
             given.\n",
        );
    }
    out.push_str(
        "\nWhen entries disagree: a rule beats a guideline, and an entry for a person beats one for their \
         audience, which beats one for everyone. This guide never lets you do more than Settings › Permissions \
         allows.\n",
    );
    if !examples.is_empty() {
        out.push_str("\nExamples of how the user writes (imitate the style, not the content):\n");
        for e in examples {
            out.push_str(&format!("<example>\n{}\n</example>\n", fenced(e)));
        }
    }
    (out, audiences)
}

/// Whether `phrase` occurs in `text` as words (any case): "circle back"
/// matches "Let's circle back." but not "encircle backs".
pub fn contains_phrase(text: &str, phrase: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    };
    let (t, p) = (words(text), words(phrase));
    !p.is_empty() && t.windows(p.len()).any(|w| w == p.as_slice())
}

/// Run the checks of the entries that apply to this message on an AI
/// draft's own text (never on text the user typed; spec §14.9). What
/// comes back is what an agent is told as `guide_check`.
pub fn check(entries: &[Entry], groups: &AudienceGroups, target: &Target, text: &str) -> Vec<CheckFailure> {
    let audiences = target.audiences_in(groups);
    let words = text.split_whitespace().count();
    entries
        .iter()
        .filter(|e| applies_in(&e.scope, target, &audiences, groups))
        // Which language a draft is in is not known here: checks scoped to
        // a language are left to the agent, which sees them in the guide.
        .filter(|e| e.scope.languages.is_empty())
        .filter_map(|e| {
            let c = e.check.as_ref()?;
            let message = match c.kind {
                CheckKind::BannedPhrase if contains_phrase(text, &c.value) => {
                    format!("Uses “{}”, which your rules ban", c.value)
                }
                CheckKind::RequiredPhrase if !contains_phrase(text, &c.value) => {
                    format!("Leaves out “{}”, which your rules require", c.value)
                }
                CheckKind::MaxWords => {
                    let limit: usize = c.value.parse().ok()?;
                    if words <= limit {
                        return None;
                    }
                    format!("Is {words} words; your guide says at most {limit}")
                }
                _ => return None,
            };
            Some(CheckFailure { entry_id: e.id, statement: e.statement.clone(), message })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_match_whole_words_in_any_case() {
        assert!(contains_phrase("Let's Circle back.", "circle back"));
        assert!(!contains_phrase("The encircle backstory", "circle back"));
        assert!(contains_phrase("Don't worry", "don't"));
        assert!(!contains_phrase("anything", ""), "an empty phrase never matches");
    }

    #[test]
    fn scopes_read_as_words() {
        let s = Scope {
            groups: vec!["Customers".into()],
            people: vec!["ann@acme.com".into()],
            message_types: vec!["reply".into()],
            languages: vec!["French".into(), "German".into()],
        };
        assert_eq!(scope_text(&s), "for Customers; to ann@acme.com; in reply; when writing in French or German");
        assert_eq!(scope_text(&Scope::default()), "");
    }

    #[test]
    fn a_max_words_check_with_no_number_is_skipped() {
        let e = Entry {
            id: 1,
            category: "A1".into(),
            kind: Kind::Rule,
            statement: "Short".into(),
            scope: Scope::default(),
            check: Some(Check { kind: CheckKind::MaxWords, value: "ten".into() }),
        };
        assert!(check(&[e], &AudienceGroups::default(), &Target::default(), "one two three").is_empty());
    }
}
