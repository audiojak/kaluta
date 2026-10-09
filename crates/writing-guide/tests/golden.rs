//! Golden tests: the exact text agents are given and the exact checks a
//! draft fails, on a representative guide and facts. The core (mailbox
//! mode, in-app tools) and the rules server both answer from this crate;
//! a change here changes what every agent reads, so it shows up as a diff.

use writing_guide::{
    AudienceGroup, AudienceGroups, Check, CheckKind, Entry, Fact, Kind, Mailbox, SCHEMA_VERSION, Scope, Snapshot,
    SnapshotError, Target, check, fact_lines, render, with_facts,
};

fn entry(id: i64, category: &str, kind: Kind, statement: &str, scope: Scope, check: Option<Check>) -> Entry {
    Entry { id, category: category.into(), kind, statement: statement.into(), scope, check }
}

fn guide() -> Vec<Entry> {
    let customers = Scope { groups: vec!["Customers".into()], ..Default::default() };
    let replies = Scope { message_types: vec!["reply".into()], ..Default::default() };
    let ann = Scope { people: vec!["ann@acme.com".into()], ..Default::default() };
    let french = Scope { languages: vec!["French".into()], ..Default::default() };
    let banned = Check { kind: CheckKind::BannedPhrase, value: "circle back".into() };
    vec![
        entry(1, "B6", Kind::Rule, "Never say 'circle back'", Scope::default(), Some(banned)),
        entry(2, "A1", Kind::Guideline, "Be warm and brief", Scope::default(), None),
        entry(3, "A2", Kind::Guideline, "Be formal", customers.clone(), None),
        entry(4, "C1", Kind::Guideline, "Answer in the first line", replies, None),
        entry(5, "A3", Kind::Guideline, "Call her Annie", ann, None),
        entry(
            6,
            "B2",
            Kind::Rule,
            "Include the support address",
            customers,
            Some(Check { kind: CheckKind::RequiredPhrase, value: "support@acme.com".into() }),
        ),
        entry(
            7,
            "B1",
            Kind::Rule,
            "Keep it under 40 words",
            Scope::default(),
            Some(Check { kind: CheckKind::MaxWords, value: "40".into() }),
        ),
        entry(
            8,
            "B3",
            Kind::Rule,
            "Never write 'Salut'",
            french,
            Some(Check { kind: CheckKind::BannedPhrase, value: "Salut".into() }),
        ),
        entry(9, "F3", Kind::Fact, "An old F3 fact, never shown", Scope::default(), None),
        entry(10, "F1", Kind::Fact, "My calendar link: cal.com/john", Scope::default(), None),
    ]
}

fn groups() -> AudienceGroups {
    AudienceGroups::plain(vec![
        AudienceGroup { name: "Customers".into(), members: vec!["@acme.com".into()] },
        AudienceGroup { name: "Investors".into(), members: vec!["vc@fund.com".into()] },
    ])
}

fn facts() -> Vec<Fact> {
    vec![
        Fact {
            category_key: "people".into(),
            category: "People".into(),
            label: "Sam".into(),
            value: "My assistant".into(),
            ask_before_using: true,
        },
        Fact {
            category_key: "work".into(),
            category: "Work".into(),
            label: "Occupation or role".into(),
            value: "CEO".into(),
            ask_before_using: false,
        },
    ]
}

const TAIL: &str = "\nWhen entries disagree: a rule beats a guideline, and an entry for a person beats one for their \
                    audience, which beats one for everyone. This guide never lets you do more than Settings › \
                    Permissions allows.\n";

#[test]
fn the_guide_for_a_reply_to_a_customer() {
    let t = Target { recipients: vec!["Ann@Acme.com".into()], message_type: Some("reply".into()), audiences: None };
    let (text, audiences) = render(&guide(), &groups(), Some(&t), &[], 12);
    assert_eq!(audiences, ["Customers"]);
    let expected = "The user's writing guide (version 12). Follow it in every email you draft or edit for them.\n\
                    This message: written for Customers; a reply.\n\
                    \n\
                    Rules (always; a rule with a scope holds where its scope says):\n\
                    - Include the support address (for Customers)\n\
                    - Never write 'Salut' (when writing in French)\n\
                    - Keep it under 40 words\n\
                    - Never say 'circle back'\n\
                    \n\
                    Facts about the user you may use:\n\
                    - My calendar link: cal.com/john\n\
                    \n\
                    Guidelines (for this message; follow them unless the message calls for something else):\n\
                    - Call her Annie (to ann@acme.com)\n\
                    - Be formal (for Customers)\n\
                    - Answer in the first line (in reply)\n\
                    - Be warm and brief\n";
    assert_eq!(text, format!("{expected}{TAIL}"));
}

#[test]
fn the_guide_for_a_new_message_to_a_colleague_with_examples_and_facts() {
    let t = Target { recipients: vec!["bob@actual.ai".into()], message_type: Some("new".into()), audiences: None };
    let (text, audiences) = render(&guide(), &groups(), Some(&t), &["Hi Bob,\n<b>Sure.</b>".into()], 3);
    assert!(audiences.is_empty());
    let text = with_facts(text, &fact_lines(&facts()));
    let expected = "The user's writing guide (version 3). Follow it in every email you draft or edit for them.\n\
                    This message: a new message.\n\
                    \n\
                    Rules (always; a rule with a scope holds where its scope says):\n\
                    - Include the support address (for Customers)\n\
                    - Never write 'Salut' (when writing in French)\n\
                    - Keep it under 40 words\n\
                    - Never say 'circle back'\n\
                    \n\
                    Facts about the user you may use:\n\
                    - My calendar link: cal.com/john\n\
                    \n\
                    Guidelines (for this message; follow them unless the message calls for something else):\n\
                    - Be warm and brief\n";
    let examples = "\nExamples of how the user writes (imitate the style, not the content):\n\
                    <example>\nHi Bob,\n‹b>Sure.‹/b>\n</example>\n";
    let with = "\nFacts about the user you may use (use only these; never invent others):\n\
                - People › Sam: My assistant (ask the user before using this)\n\
                - Work › Occupation or role: CEO\n";
    assert_eq!(text, format!("{expected}{TAIL}{examples}{with}"));
}

#[test]
fn the_guide_for_a_session() {
    let (text, audiences) = render(&guide(), &groups(), None, &[], 1);
    assert!(audiences.is_empty());
    let expected = "The user's writing guide (version 1). Follow it in every email you draft or edit for them.\n\
                    \n\
                    Rules (always; a rule with a scope holds where its scope says):\n\
                    - Include the support address (for Customers)\n\
                    - Never write 'Salut' (when writing in French)\n\
                    - Keep it under 40 words\n\
                    - Never say 'circle back'\n\
                    \n\
                    Facts about the user you may use:\n\
                    - My calendar link: cal.com/john\n\
                    \n\
                    Guidelines (each where its scope says; follow them unless the message calls for something else):\n\
                    - Call her Annie (to ann@acme.com)\n\
                    - Be formal (for Customers)\n\
                    - Answer in the first line (in reply)\n\
                    - Be warm and brief\n";
    assert_eq!(text, format!("{expected}{TAIL}"));
}

#[test]
fn no_guide_renders_nothing_and_facts_alone_stand() {
    assert_eq!(render(&[], &groups(), None, &[], 4), (String::new(), vec![]));
    let only_f3 = vec![entry(1, "F3", Kind::Fact, "Old", Scope::default(), None)];
    assert_eq!(render(&only_f3, &groups(), None, &[], 4).0, "");
    assert_eq!(
        with_facts(String::new(), &fact_lines(&facts())),
        "Facts about the user you may use (use only these; never invent others):\n\
         - People › Sam: My assistant (ask the user before using this)\n\
         - Work › Occupation or role: CEO\n"
    );
}

#[test]
fn checks_on_representative_drafts() {
    let to_customer =
        Target { recipients: vec!["ann@acme.com".into()], message_type: Some("reply".into()), audiences: None };
    let long = "word ".repeat(41);
    let failures = check(&guide(), &groups(), &to_customer, &format!("Salut Ann, let's Circle Back. {long}"));
    let got: Vec<(i64, &str, &str)> =
        failures.iter().map(|f| (f.entry_id, f.statement.as_str(), f.message.as_str())).collect();
    assert_eq!(
        got,
        [
            (1, "Never say 'circle back'", "Uses “circle back”, which your rules ban"),
            (6, "Include the support address", "Leaves out “support@acme.com”, which your rules require"),
            (7, "Keep it under 40 words", "Is 46 words; your guide says at most 40"),
        ],
        "the French check is left to the agent"
    );

    let fine = "Hi Ann, write to support@acme.com and we will help.";
    assert!(check(&guide(), &groups(), &to_customer, fine).is_empty());
    let to_colleague = Target { recipients: vec!["bob@actual.ai".into()], ..to_customer.clone() };
    assert!(check(&guide(), &groups(), &to_colleague, "Sure.").is_empty(), "scoped checks stay in scope");
    let chosen = Target { audiences: Some(vec!["customers".into()]), ..to_colleague };
    assert_eq!(check(&guide(), &groups(), &chosen, "Sure.").len(), 1, "an audience chosen for the draft");
}

fn snapshot() -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        version: 5,
        guide_version: 12,
        published_at: 1_760_000_000_000,
        mailbox: Mailbox {
            address: "outreach@agents.example".into(),
            name: "Outreach".into(),
            about: "You send as Outreach.".into(),
        },
        entries: guide().into_iter().filter(|e| [1, 3, 6].contains(&e.id)).collect(),
        audiences: groups().hashed("pepper"),
        facts: facts().into_iter().skip(1).collect(),
    }
}

#[test]
fn the_snapshot_json_shape_is_pinned() {
    let json = snapshot().to_json().unwrap();
    let customers = writing_guide::hash_address("pepper", "@acme.com");
    let investor = writing_guide::hash_address("pepper", "vc@fund.com");
    let expected = format!(
        r#"{{"schema_version":1,"version":5,"guide_version":12,"published_at":1760000000000,"mailbox":{{"address":"outreach@agents.example","name":"Outreach","about":"You send as Outreach."}},"entries":[{{"id":1,"category":"B6","kind":"rule","statement":"Never say 'circle back'","scope":{{"groups":[],"people":[],"message_types":[],"languages":[]}},"check":{{"kind":"banned_phrase","value":"circle back"}}}},{{"id":3,"category":"A2","kind":"guideline","statement":"Be formal","scope":{{"groups":["Customers"],"people":[],"message_types":[],"languages":[]}},"check":null}},{{"id":6,"category":"B2","kind":"rule","statement":"Include the support address","scope":{{"groups":["Customers"],"people":[],"message_types":[],"languages":[]}},"check":{{"kind":"required_phrase","value":"support@acme.com"}}}}],"audiences":{{"salt":"pepper","groups":[{{"name":"Customers","members":["{customers}"]}},{{"name":"Investors","members":["{investor}"]}}]}},"facts":[{{"category_key":"work","category":"Work","label":"Occupation or role","value":"CEO","ask_before_using":false}}]}}"#
    );
    assert_eq!(json, expected);
    assert!(!json.contains("acme.com\"]") && !json.contains("vc@fund.com"), "no address leaves as written");
    assert_eq!(Snapshot::from_json(&json).unwrap(), snapshot(), "it reads back as written");
}

#[test]
fn a_snapshot_answers_as_the_app_does() {
    let s = snapshot();
    let t = Target { recipients: vec!["ann@acme.com".into()], message_type: Some("reply".into()), audiences: None };
    let (text, audiences) = render(&s.entries, &groups(), Some(&t), &[], 12);
    let rendered = s.guide(Some(&t));
    assert_eq!(rendered.audiences, audiences, "hashed members match as plain ones");
    assert_eq!(rendered.text, with_facts(text, &fact_lines(&s.facts)));
    assert_eq!(rendered.version, 12);
    assert!(rendered.text.contains("- Be formal (for Customers)"));
    let failures = s.check(&t, "Let's circle back.");
    assert_eq!(failures.iter().map(|f| f.entry_id).collect::<Vec<_>>(), [1, 6]);
    assert_eq!(check(&s.entries, &groups(), &t, "Let's circle back."), failures);
    assert_eq!(
        s.facts_lookup(Some("WORK"), Some("ceo")),
        serde_json::json!({"facts": [{"category": "Work", "label": "Occupation or role", "value": "CEO", "ask_before_using": false}]})
    );
    assert_eq!(s.facts_lookup(Some("people"), None), serde_json::json!({"facts": []}));
}

#[test]
fn facts_lookup_matches_key_or_name_and_any_part() {
    let f = facts();
    let all = writing_guide::facts_lookup(&f, None, None);
    assert_eq!(all["facts"].as_array().unwrap().len(), 2);
    assert_eq!(writing_guide::facts_lookup(&f, Some("people"), None)["facts"][0]["label"], "Sam");
    assert_eq!(writing_guide::facts_lookup(&f, Some("People"), None)["facts"][0]["ask_before_using"], true);
    assert_eq!(writing_guide::facts_lookup(&f, None, Some("ASSISTANT"))["facts"][0]["value"], "My assistant");
    assert_eq!(writing_guide::facts_lookup(&f, None, Some("work occ"))["facts"][0]["value"], "CEO");
    assert_eq!(writing_guide::facts_lookup(&f, None, Some("  "))["facts"].as_array().unwrap().len(), 2);
}

#[test]
fn snapshots_from_another_schema_or_with_addresses_are_refused() {
    let mut v: serde_json::Value = serde_json::from_str(&snapshot().to_json().unwrap()).unwrap();
    v["schema_version"] = 2.into();
    assert!(matches!(Snapshot::from_json(&v.to_string()), Err(SnapshotError::UnsupportedSchema(2))));
    v.as_object_mut().unwrap().remove("schema_version");
    assert!(matches!(Snapshot::from_json(&v.to_string()), Err(SnapshotError::MissingSchema)));

    let mut plain = snapshot();
    plain.audiences = groups();
    let json = plain.to_json().unwrap();
    assert!(matches!(Snapshot::from_json(&json), Err(SnapshotError::PlainAddresses)));
    assert!(matches!(Snapshot::from_json("[]"), Err(SnapshotError::MissingSchema)));
    assert!(matches!(Snapshot::from_json("{"), Err(SnapshotError::Malformed(_))));
}

/// The guide as published: [`guide`] plus a rule for one company's people,
/// every address hashed.
fn guide_with_people() -> Vec<Entry> {
    let globex = Scope { people: vec!["@globex.com".into()], ..Default::default() };
    let pricing = Check { kind: CheckKind::BannedPhrase, value: "pricing".into() };
    let mut entries = guide();
    entries.push(entry(11, "B4", Kind::Rule, "Never mention pricing", globex, Some(pricing)));
    entries
}

fn published_with_people() -> Snapshot {
    Snapshot { entries: guide_with_people(), audiences: groups(), ..snapshot() }.hashed("pepper")
}

#[test]
fn people_in_scopes_are_published_as_hashes() {
    let s = published_with_people();
    let json = s.to_json().unwrap();
    assert!(!json.contains("ann@acme.com") && !json.contains("globex.com"), "{json}");
    let ann = writing_guide::hash_address("pepper", "ann@acme.com");
    let globex = writing_guide::hash_address("pepper", "@globex.com");
    assert!(json.contains(&format!(r#""people":["{ann}"]"#)) && json.contains(&format!(r#""people":["{globex}"]"#)));
    assert_eq!(Snapshot::from_json(&json).unwrap(), s, "it reads back as written");
    assert_eq!(s.clone().hashed("other"), s, "hashing twice changes nothing");

    let mut plain_person: serde_json::Value = serde_json::from_str(&json).unwrap();
    plain_person["entries"][4]["scope"]["people"] = serde_json::json!(["ann@acme.com"]);
    assert!(matches!(Snapshot::from_json(&plain_person.to_string()), Err(SnapshotError::PlainAddresses)));
    plain_person["audiences"] = serde_json::json!({ "groups": [] });
    plain_person["entries"][4]["scope"]["people"] = serde_json::json!([ann]);
    assert!(
        matches!(Snapshot::from_json(&plain_person.to_string()), Err(SnapshotError::PlainAddresses)),
        "a hash with no salt cannot be matched, and may be an address"
    );
}

#[test]
fn a_published_entry_for_a_person_reads_as_the_apps_when_writing_to_them() {
    let s = published_with_people();
    let t = Target { recipients: vec!["Ann@Acme.com".into()], message_type: Some("reply".into()), audiences: None };
    let (app, _) = render(&guide_with_people(), &groups(), Some(&t), &[], 12);
    let published = s.guide(Some(&t));
    // The app also shows the rule for Globex's people, with their domain;
    // the published guide cannot name it, and leaves it out.
    assert_eq!(
        published.text,
        with_facts(app.replace("- Never mention pricing (to @globex.com)\n", ""), &fact_lines(&s.facts))
    );
    assert!(published.text.contains("- Call her Annie (to ann@acme.com)\n"));
    assert!(s.check(&t, "Our pricing is fair, support@acme.com").is_empty(), "the rule for Globex is not Ann's");
}

#[test]
fn a_published_entry_for_a_domain_names_the_recipient_it_matched() {
    let s = published_with_people();
    let t = Target {
        recipients: vec!["bea@globex.com".into(), "vc@fund.com".into()],
        message_type: Some("new".into()),
        audiences: None,
    };
    let rendered = s.guide(Some(&t));
    assert_eq!(rendered.audiences, ["Investors"]);
    let expected = "The user's writing guide (version 12). Follow it in every email you draft or edit for them.\n\
                    This message: written for Investors; a new message.\n\
                    \n\
                    Rules (always; a rule with a scope holds where its scope says):\n\
                    - Never mention pricing (to bea@globex.com)\n\
                    - Include the support address (for Customers)\n\
                    - Never write 'Salut' (when writing in French)\n\
                    - Keep it under 40 words\n\
                    - Never say 'circle back'\n\
                    \n\
                    Facts about the user you may use:\n\
                    - My calendar link: cal.com/john\n\
                    \n\
                    Guidelines (for this message; follow them unless the message calls for something else):\n\
                    - Be warm and brief\n";
    let facts = "\nFacts about the user you may use (use only these; never invent others):\n\
                 - Work › Occupation or role: CEO\n";
    assert_eq!(rendered.text, format!("{expected}{TAIL}{facts}"));
    let failures = s.check(&t, "About pricing.");
    assert_eq!(failures.iter().map(|f| f.entry_id).collect::<Vec<_>>(), [11]);

    // A rule for a person is listed whenever they are written to, with the
    // rest of its scope, as the app lists every rule.
    let mut narrow = published_with_people();
    narrow.entries[10].scope.message_types = vec!["reply".into()];
    assert!(narrow.guide(Some(&t)).text.contains("- Never mention pricing (to bea@globex.com; in reply)\n"));
    assert!(narrow.check(&t, "About pricing.").is_empty(), "its check holds only in a reply");
}

#[test]
fn a_published_guide_with_no_recipients_leaves_out_entries_for_people() {
    let s = published_with_people();
    let expected = "The user's writing guide (version 12). Follow it in every email you draft or edit for them.\n\
                    \n\
                    Rules (always; a rule with a scope holds where its scope says):\n\
                    - Include the support address (for Customers)\n\
                    - Never write 'Salut' (when writing in French)\n\
                    - Keep it under 40 words\n\
                    - Never say 'circle back'\n\
                    \n\
                    Facts about the user you may use:\n\
                    - My calendar link: cal.com/john\n\
                    \n\
                    Guidelines (each where its scope says; follow them unless the message calls for something else):\n\
                    - Be formal (for Customers)\n\
                    - Answer in the first line (in reply)\n\
                    - Be warm and brief\n\
                    \n\
                    Some entries are for particular people and are shown only when the message's recipients are \
                    given.\n";
    let facts = "\nFacts about the user you may use (use only these; never invent others):\n\
                 - Work › Occupation or role: CEO\n";
    assert_eq!(s.guide(None).text, format!("{expected}{TAIL}{facts}"));
    let no_one = Target { message_type: Some("new".into()), ..Default::default() };
    assert!(s.guide(Some(&no_one)).text.contains("Some entries are for particular people"));
    assert!(!s.guide(Some(&no_one)).text.contains("Annie"));
}
