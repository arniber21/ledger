//! Replay authoring. Gemini and Jev come from saved fixtures.
//!
//! A missing fixture is a failure. This module does not call Gemini or System
//! One, and it does not invent a Same match.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ledger_core::AgentId;
use ledger_jev::{ForcedOutcome, JevCallFailure, ReplayCatalog};

use crate::author::{author, AuthoringInput, AuthoringOutcome};
use crate::clause::{coverage_request, split_clauses};
use crate::draft::{draft_from_interaction, draft_request, PatternDraft};

/// Policy whose saved draft names a pattern the catalogue does not have.
pub const CATALOGUE_REJECT_POLICY: &str = "Do not arm a pattern that is outside the catalogue.";

/// Two clauses. The saved coverage marks the second clause uncovered.
pub const TWO_CLAUSE_POLICY: &str = "Pay the bill. Keep a credit.";

/// Runs [`author`] on saved Gemini and Jev bodies.
///
/// `force` replaces the Jev fixtures when the test API has armed an outcome.
/// Timeout is a call failure. A missing Gemini fixture rejects the draft.
/// A missing Jev fixture is a call failure, which maps to Unknown and does
/// not arm. Neither case calls out.
#[must_use = "the outcome holds the running monitor"]
pub fn author_replay(
    sentence: &str,
    agent: &AgentId,
    override_coverage: bool,
    force: Option<ForcedOutcome>,
) -> AuthoringOutcome {
    let Some(gemini) = load_gemini(sentence) else {
        return author(AuthoringInput {
            sentence,
            agent,
            gemini: Err(crate::DraftFailure::Transport),
            jev: Err(JevCallFailure::Transport),
            coverage: Err(JevCallFailure::Transport),
            override_coverage,
        });
    };
    let (jev, coverage) = match draft_from_interaction(&gemini) {
        Some(draft) => judgments(sentence, &draft, force),
        None => (
            Err(JevCallFailure::Transport),
            Err(JevCallFailure::Transport),
        ),
    };
    author(AuthoringInput {
        sentence,
        agent,
        gemini: Ok(&gemini),
        jev: borrow(&jev),
        coverage: borrow(&coverage),
        override_coverage,
    })
}

fn load_gemini(sentence: &str) -> Option<String> {
    let catalog = GeminiCatalog::load_default().ok()?;
    catalog.require(&draft_request(sentence)).map(str::to_owned)
}

fn judgments(
    sentence: &str,
    draft: &PatternDraft,
    force: Option<ForcedOutcome>,
) -> (
    Result<String, JevCallFailure>,
    Result<String, JevCallFailure>,
) {
    let jev = judgment(&crate::match_request(sentence, draft), force);
    let coverage = if split_clauses(sentence).len() <= 1 {
        Err(JevCallFailure::Timeout)
    } else {
        judgment(
            &coverage_request(sentence, &split_clauses(sentence), draft),
            force,
        )
    };
    (jev, coverage)
}

fn judgment(request: &str, force: Option<ForcedOutcome>) -> Result<String, JevCallFailure> {
    if let Some(outcome) = force {
        return outcome.apply(request);
    }
    let catalog = ReplayCatalog::load_default().map_err(|_| JevCallFailure::Transport)?;
    catalog
        .require(request)
        .map(str::to_owned)
        .map_err(|_| JevCallFailure::Transport)
}

fn borrow(result: &Result<String, JevCallFailure>) -> Result<&str, JevCallFailure> {
    match result {
        Ok(body) => Ok(body.as_str()),
        Err(error) => Err(*error),
    }
}

/// Saved Gemini Interactions bodies, keyed by the exact draft request.
struct GeminiCatalog {
    bodies: BTreeMap<String, String>,
}

impl GeminiCatalog {
    fn load_default() -> Result<Self, std::io::Error> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gemini");
        Self::load(&dir)
    }

    fn load(dir: &Path) -> Result<Self, std::io::Error> {
        let mut files = Vec::new();
        walk(dir, &mut files);
        let mut requests: BTreeMap<(PathBuf, String), String> = BTreeMap::new();
        let mut responses: BTreeMap<(PathBuf, String), String> = BTreeMap::new();
        for path in files {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(parent) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            let Some((stem, is_request)) = stem_kind(name) else {
                continue;
            };
            let text = fs::read_to_string(&path)?;
            let key = (parent, stem.to_owned());
            if is_request {
                requests.insert(key, text);
            } else {
                responses.entry(key).or_insert(text);
            }
        }
        let mut bodies = BTreeMap::new();
        for (key, request) in requests {
            let Some(response) = responses.get(&key) else {
                continue;
            };
            bodies.insert(request, response.clone());
        }
        Ok(Self { bodies })
    }

    fn require(&self, request: &str) -> Option<&str> {
        self.bodies.get(request).map(String::as_str)
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}

fn stem_kind(name: &str) -> Option<(&str, bool)> {
    if let Some(stem) = name.strip_suffix("-request.json") {
        return Some((stem, true));
    }
    if let Some(stem) = name.strip_suffix("-response.json") {
        return Some((stem, false));
    }
    let stem = name.strip_suffix(".json")?;
    if stem.ends_with("-request") || stem.ends_with("-response") {
        return None;
    }
    Some((stem, false))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{AuthoringStep, HoldReason, RejectReason, FIXTURE_POLICY};
    use ledger_jev::BindingVerdict;

    fn agent() -> AgentId {
        AgentId::new("payer").expect("agent")
    }

    #[test]
    fn saved_clause_and_catalogue_fixtures_match_the_builders() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let read = |path: &str| fs::read_to_string(root.join(path)).expect(path);
        assert_eq!(
            read("gemini/catalogue-reject-request.json"),
            draft_request(CATALOGUE_REJECT_POLICY)
        );
        let rejected =
            draft_from_interaction(&read("gemini/catalogue-reject.json")).expect("draft");
        assert_eq!(rejected.pattern_id(), "not-a-pattern");
        assert_eq!(
            read("gemini/two-clause-request.json"),
            draft_request(TWO_CLAUSE_POLICY)
        );
        let draft = draft_from_interaction(&read("gemini/two-clause.json")).expect("draft");
        assert_eq!(draft.pattern_id(), "absence.after-until");
        assert_eq!(
            read("jev/two-clause-match-request.json"),
            crate::match_request(TWO_CLAUSE_POLICY, &draft)
        );
        assert_eq!(
            ledger_jev::map_choice_body(&read("jev/two-clause-match.json"), "draft_matches_policy"),
            BindingVerdict::Same
        );
        let clauses = split_clauses(TWO_CLAUSE_POLICY);
        assert_eq!(
            read("jev/two-clause-coverage-request.json"),
            coverage_request(TWO_CLAUSE_POLICY, &clauses, &draft)
        );
        let coverage = read("jev/two-clause-coverage.json");
        assert_eq!(
            ledger_jev::map_choice_body(&coverage, "clause_0"),
            BindingVerdict::Same
        );
        assert_eq!(
            ledger_jev::map_choice_body(&coverage, "clause_1"),
            BindingVerdict::Different
        );
        for path in [
            "gemini/catalogue-reject.json",
            "gemini/two-clause.json",
            "jev/two-clause-match.json",
            "jev/two-clause-coverage.json",
        ] {
            let raw = read(path);
            assert!(!raw.contains("GEMINI_API_KEY"));
            assert!(!raw.contains("TYPESAFE_API_KEY"));
            assert!(!raw.to_ascii_lowercase().contains("explain"));
        }
    }

    #[test]
    fn fixture_policy_arms_from_replay() {
        let outcome = author_replay(FIXTURE_POLICY, &agent(), false, None);
        assert!(outcome.monitor().is_some());
        assert!(matches!(
            outcome.steps().last(),
            Some(AuthoringStep::Accepted {
                overridden: false,
                ..
            })
        ));
        let sse = outcome.encode_sse();
        assert!(!sse.to_ascii_lowercase().contains("fraud"));
        assert!(!sse.contains("explanation"));
    }

    #[test]
    fn unknown_pattern_is_rejected_from_its_fixture() {
        let outcome = author_replay(CATALOGUE_REJECT_POLICY, &agent(), false, None);
        assert!(outcome.monitor().is_none());
        assert!(matches!(
            outcome.steps().last(),
            Some(AuthoringStep::Rejected {
                reason: RejectReason::UnknownPattern
            })
        ));
        assert!(!outcome
            .steps()
            .iter()
            .any(|step| matches!(step, AuthoringStep::Check { .. })));
    }

    #[test]
    fn two_clauses_hold_until_override() {
        let held = author_replay(TWO_CLAUSE_POLICY, &agent(), false, None);
        assert!(held.monitor().is_none());
        assert!(matches!(
            held.steps().last(),
            Some(AuthoringStep::Held {
                reason: HoldReason::ClauseUncovered
            })
        ));
        let armed = author_replay(TWO_CLAUSE_POLICY, &agent(), true, None);
        assert!(armed.monitor().is_some());
        assert!(matches!(
            armed.steps().last(),
            Some(AuthoringStep::Accepted {
                overridden: true,
                ..
            })
        ));
    }

    #[test]
    fn a_missing_fixture_rejects_and_does_not_arm() {
        let outcome = author_replay("This sentence has no saved draft.", &agent(), false, None);
        assert!(outcome.monitor().is_none());
        assert!(matches!(
            outcome.steps().last(),
            Some(AuthoringStep::Rejected {
                reason: RejectReason::DraftUnavailable
            })
        ));
    }

    #[test]
    fn forced_different_and_unknown_do_not_arm() {
        let different = author_replay(
            FIXTURE_POLICY,
            &agent(),
            false,
            Some(ForcedOutcome::Different),
        );
        assert!(different.monitor().is_none());
        assert!(matches!(
            different.steps().last(),
            Some(AuthoringStep::Rejected {
                reason: RejectReason::DraftDiffers
            })
        ));
        let unknown = author_replay(
            FIXTURE_POLICY,
            &agent(),
            false,
            Some(ForcedOutcome::Unknown),
        );
        assert!(unknown.monitor().is_none());
        assert!(matches!(
            unknown.steps(),
            [
                AuthoringStep::Sentence { .. },
                AuthoringStep::Draft { .. },
                AuthoringStep::Check {
                    verdict: BindingVerdict::Unknown
                },
                AuthoringStep::Coverage { .. },
                AuthoringStep::Rejected {
                    reason: RejectReason::MatchInconclusive
                }
            ]
        ));
        let timeout = author_replay(
            FIXTURE_POLICY,
            &agent(),
            false,
            Some(ForcedOutcome::Timeout),
        );
        assert!(timeout.monitor().is_none());
        assert!(matches!(
            timeout.steps().last(),
            Some(AuthoringStep::Rejected {
                reason: RejectReason::MatchInconclusive
            })
        ));
    }
}
