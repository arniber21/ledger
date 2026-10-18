//! Clause coverage for one drafted Pattern.
//!
//! Code splits the policy into clauses. One System One request then asks a
//! Choice per clause, and [`ledger_jev::map_choice_exchange`] maps each answer
//! at the fixed 0.80 thresholds. Same covers the clause. Different leaves it
//! uncovered. Timeout, error, contradiction, or a score under the threshold
//! is Unknown. This module does not set its own thresholds and does not ask
//! for explanation text.

use ledger_jev::{map_choice_exchange, BindingVerdict, JevCallFailure};
use sentinel_compile::catalogue::catalogue;
use serde_json::{Map, Value};

use crate::check::DRAFT_MATCH_QUESTION;
use crate::draft::{parameters_json, sort_objects, PatternDraft};

/// How a drafted Pattern relates to one clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ClauseCoverage {
    /// Jev returned Same at or above the fixed threshold.
    Covered,
    /// Jev returned Different at or above the fixed threshold.
    Uncovered,
    /// Timeout, error, contradiction, or a score under the threshold.
    Unknown,
}

/// Why arming is held after a Same draft match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum HoldReason {
    /// At least one clause is uncovered.
    ClauseUncovered,
    /// No clause is uncovered, and at least one clause is Unknown.
    ClauseUnknown,
}

/// One clause and the Jev question that judged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedClause {
    /// Slice of the policy. The slices concatenate to the policy.
    pub text: String,
    /// Mapped coverage. Not a model explanation.
    pub coverage: ClauseCoverage,
    /// System One question id whose Choice produced [`Self::coverage`].
    pub question_id: String,
}

/// Question id for clause `index` in a batched coverage request.
///
/// The id is for this crate. The model sees the question instructions.
#[must_use]
pub fn clause_question_id(index: usize) -> String {
    format!("clause_{index}")
}

/// Splits `sentence` into clauses that concatenate back to `sentence`.
///
/// Boundaries are sentence punctuation followed by whitespace, and the
/// conjunctions `and`, `but`, and `or` when they stand as their own words.
/// A decimal point is not a boundary. An empty sentence yields no clauses.
#[must_use]
pub fn split_clauses(sentence: &str) -> Vec<String> {
    if sentence.is_empty() {
        return Vec::new();
    }
    let mut cuts = vec![0usize];
    let chars: Vec<(usize, char)> = sentence.char_indices().collect();
    for (index, (_, ch)) in chars.iter().enumerate() {
        if !matches!(ch, '.' | '!' | '?' | ';') {
            continue;
        }
        let Some((_, next)) = chars.get(index + 1) else {
            continue;
        };
        if !next.is_whitespace() {
            continue;
        }
        let mut rest = index + 1;
        while rest < chars.len() && chars[rest].1.is_whitespace() {
            rest += 1;
        }
        if chars.get(rest).is_some_and(|(_, ch)| ch.is_alphanumeric()) {
            cuts.push(chars[rest].0);
        }
    }
    for conjunction in [" and ", " but ", " or "] {
        let mut from = 0;
        while let Some(found) = sentence[from..].find(conjunction) {
            let at = from + found;
            let next = at + conjunction.len();
            let left_has_word = sentence[..at].chars().any(char::is_alphanumeric);
            let right_starts = sentence[next..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
            if left_has_word && right_starts {
                cuts.push(next);
            }
            from = next;
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut clauses = Vec::with_capacity(cuts.len());
    for (index, start) in cuts.iter().enumerate() {
        let end = cuts.get(index + 1).copied().unwrap_or(sentence.len());
        if *start < end {
            clauses.push(sentence[*start..end].to_owned());
        }
    }
    clauses
}

/// Body of one batched `POST https://api.typesafe.ai/v1/systemone` coverage call.
///
/// One Choice question per clause. The body has no API key.
#[must_use]
pub fn coverage_request(sentence: &str, clauses: &[String], draft: &PatternDraft) -> String {
    let value = sort_objects(coverage_request_value(sentence, clauses, draft));
    serde_json::to_string(&value).expect("coverage request is finite JSON")
}

/// Maps each clause through the shared Jev client.
///
/// A single clause uses the draft-match verdict: that clause is the whole
/// policy, so the match Choice is its coverage. Two or more clauses use
/// `coverage`, one question id per clause. A failed coverage call marks every
/// one of those clauses [`ClauseCoverage::Unknown`].
#[must_use]
pub fn judge_coverage(
    clauses: &[String],
    match_verdict: BindingVerdict,
    coverage: Result<&str, JevCallFailure>,
) -> Vec<MarkedClause> {
    if clauses.len() <= 1 {
        return clauses
            .first()
            .map(|text| MarkedClause {
                text: text.clone(),
                coverage: coverage_from_verdict(match_verdict),
                question_id: DRAFT_MATCH_QUESTION.to_owned(),
            })
            .into_iter()
            .collect();
    }
    clauses
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let question_id = clause_question_id(index);
            let verdict = map_choice_exchange(coverage, &question_id);
            MarkedClause {
                text: text.clone(),
                coverage: coverage_from_verdict(verdict),
                question_id,
            }
        })
        .collect()
}

/// Returns why a Same match must not arm, when coverage is incomplete.
///
/// Uncovered wins over Unknown when both are present. Every clause covered
/// returns [`None`].
#[must_use]
pub fn hold_reason(clauses: &[MarkedClause]) -> Option<HoldReason> {
    if clauses
        .iter()
        .any(|clause| clause.coverage == ClauseCoverage::Uncovered)
    {
        return Some(HoldReason::ClauseUncovered);
    }
    if clauses.is_empty()
        || clauses
            .iter()
            .any(|clause| clause.coverage == ClauseCoverage::Unknown)
    {
        return Some(HoldReason::ClauseUnknown);
    }
    None
}

pub(crate) fn coverage_label(coverage: ClauseCoverage) -> &'static str {
    match coverage {
        ClauseCoverage::Covered => "covered",
        ClauseCoverage::Uncovered => "uncovered",
        ClauseCoverage::Unknown => "unknown",
    }
}

pub(crate) fn verdict_label(coverage: ClauseCoverage) -> &'static str {
    match coverage {
        ClauseCoverage::Covered => "Same",
        ClauseCoverage::Uncovered => "Different",
        ClauseCoverage::Unknown => "Unknown",
    }
}

pub(crate) fn hold_label(reason: HoldReason) -> &'static str {
    match reason {
        HoldReason::ClauseUncovered => "clause_uncovered",
        HoldReason::ClauseUnknown => "clause_unknown",
    }
}

fn coverage_from_verdict(verdict: BindingVerdict) -> ClauseCoverage {
    match verdict {
        BindingVerdict::Same => ClauseCoverage::Covered,
        BindingVerdict::Different => ClauseCoverage::Uncovered,
        BindingVerdict::Unknown => ClauseCoverage::Unknown,
    }
}

fn coverage_request_value(sentence: &str, clauses: &[String], draft: &PatternDraft) -> Value {
    let summary = catalogue()
        .iter()
        .find(|item| item.id.as_str() == draft.pattern_id())
        .map(|item| item.summary)
        .unwrap_or("This pattern_id is not in the catalogue.");
    let mut questions = Map::new();
    for (index, _clause) in clauses.iter().enumerate() {
        questions.insert(
            clause_question_id(index),
            serde_json::json!({
                "type": "choice",
                "instructions": format!(
                    "Does `pattern_id` with `parameters` cover clause {index} in `clauses`, or leave that clause out? `catalogue_summary` is what that pattern_id means."
                ),
                "criteria": {
                    "same": {
                        "what": "The draft covers this clause.",
                        "still_same": "Wording can differ when the pattern, the events, and the binding still cover the clause.",
                        "not_for": "Do not pick this when the draft leaves the clause out."
                    },
                    "different": {
                        "what": "The draft leaves this clause out.",
                        "still_different": "A clause the pattern does not state is out.",
                        "not_for": "Do not pick this for wording alone when the clause is covered."
                    }
                }
            }),
        );
    }
    serde_json::json!({
        "state": {
            "policy": sentence,
            "clauses": clauses,
            "pattern_id": draft.pattern_id(),
            "parameters": parameters_json(draft),
            "catalogue_summary": summary
        },
        "model": "jev-latest",
        "questions": questions
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_policy_is_one_clause() {
        let clauses = split_clauses(crate::draft::FIXTURE_POLICY);
        assert_eq!(clauses, vec![crate::draft::FIXTURE_POLICY.to_owned()]);
    }

    #[test]
    fn two_sentences_split_and_concatenate() {
        let sentence = "Pay the bill. Keep a credit.";
        let clauses = split_clauses(sentence);
        assert_eq!(
            clauses,
            vec!["Pay the bill. ".to_owned(), "Keep a credit.".to_owned()]
        );
        assert_eq!(clauses.concat(), sentence);
    }

    #[test]
    fn conjunction_and_decimal_follow_the_same_rule() {
        let sentence = "Use 0.80 and file the receipt.";
        let clauses = split_clauses(sentence);
        assert_eq!(
            clauses,
            vec!["Use 0.80 and ".to_owned(), "file the receipt.".to_owned()]
        );
        assert_eq!(clauses.concat(), sentence);
        assert_eq!(
            split_clauses("Keep the rate at 0.80 until a credit."),
            vec!["Keep the rate at 0.80 until a credit.".to_owned()]
        );
    }
}
