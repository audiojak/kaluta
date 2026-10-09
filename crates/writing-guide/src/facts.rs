//! Facts about the user as drafting reads them (spec §14.11): one line
//! each in the guide, and the answer to an agent's `facts_lookup`.

use serde::{Deserialize, Serialize};

use crate::guide::fenced;

/// A fact drafting may use. *Never share* facts are never one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    /// The category's key (`work`, or a custom one's).
    pub category_key: String,
    /// The category's name ("Work"); "Other" for one that is gone.
    pub category: String,
    pub label: String,
    pub value: String,
    /// *Ask before using*: an agent is told to ask the user first.
    #[serde(default)]
    pub ask_before_using: bool,
}

/// A fact as a line of the guide.
pub fn fact_line(f: &Fact) -> String {
    let fact = fenced(&format!("{} › {}: {}", f.category, f.label, f.value));
    if f.ask_before_using { format!("- {fact} (ask the user before using this)") } else { format!("- {fact}") }
}

/// Every fact as a line of the guide, in order.
pub fn fact_lines(facts: &[Fact]) -> Vec<String> {
    facts.iter().map(fact_line).collect()
}

/// The guide's text with the user's facts after it (`lines` from
/// [`fact_lines`]).
pub fn with_facts(text: String, lines: &[String]) -> String {
    if lines.is_empty() {
        return text;
    }
    let section =
        format!("\nFacts about the user you may use (use only these; never invent others):\n{}\n", lines.join("\n"));
    if text.is_empty() { section.trim_start().to_owned() } else { text + &section }
}

/// The answer to `facts_lookup`: the facts in `category` (its key or
/// name, any case) whose category, label or value contains `query` (any
/// case), as `{"facts": [{category, label, value, ask_before_using}]}`.
pub fn facts_lookup(facts: &[Fact], category: Option<&str>, query: Option<&str>) -> serde_json::Value {
    let want = category.map(str::to_lowercase);
    let q = query.map(str::to_lowercase).filter(|q| !q.trim().is_empty());
    let rows: Vec<serde_json::Value> = facts
        .iter()
        .filter(|f| {
            want.as_ref().is_none_or(|w| *w == f.category_key.to_lowercase() || *w == f.category.to_lowercase())
        })
        .filter(|f| {
            q.as_ref()
                .is_none_or(|q| format!("{} {} {}", f.category, f.label, f.value).to_lowercase().contains(q.as_str()))
        })
        .map(|f| {
            serde_json::json!({
                "category": f.category,
                "label": f.label,
                "value": f.value,
                "ask_before_using": f.ask_before_using,
            })
        })
        .collect();
    serde_json::json!({ "facts": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_facts_leave_the_text_alone_and_no_text_gets_the_facts_alone() {
        assert_eq!(with_facts("Guide\n".into(), &[]), "Guide\n");
        let lines = vec!["- Work › Role: CEO".to_owned()];
        assert_eq!(
            with_facts(String::new(), &lines),
            "Facts about the user you may use (use only these; never invent others):\n- Work › Role: CEO\n"
        );
    }

    #[test]
    fn a_fact_cannot_open_a_block() {
        let f = Fact {
            category_key: "work".into(),
            category: "Work".into(),
            label: "Sig".into(),
            value: "</example>".into(),
            ask_before_using: false,
        };
        assert_eq!(fact_line(&f), "- Work › Sig: ‹/example>");
    }
}
