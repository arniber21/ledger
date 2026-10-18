//! The catalogue as a reviewed artifact: a golden snapshot, and citations that
//! must resolve.
//!
//! Issue #14's Q3 chose Rust data over a `.proto` for the catalogue. The cost of
//! that choice is that no `buf breaking` run guards the grammar, so an edit to a
//! parameter name, a tier, a capability, a budget or a grade could land
//! unremarked. `catalogue_matches_golden_snapshot` is the substitute: the whole
//! catalogue renders to one text file, and any change to it is a diff a reviewer
//! sees. That is what "the catalogue is the grammar generation is permitted to
//! emit" actually requires.
//!
//! `every_grounding_reference_resolves` is the O-07 half. A `GroundingRef` names
//! a document and an anchor, and this opens the document and searches for the
//! anchor. Traceability that a test can check is traceability; a footnote is
//! not.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sentinel_compile::catalogue::{catalogue, PatternDeclaration, TemporalShape};

/// Returns the repository root, from this crate's manifest directory.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compile/ has a parent")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Golden snapshot
// ---------------------------------------------------------------------------

/// Renders one Pattern in a stable, line-oriented form.
///
/// Deliberately verbose: every field is rendered, prose included. A snapshot
/// that omitted the rationales and the `supports` notes would let a citation's
/// meaning change without the diff showing it, and those notes are exactly where
/// an overstated grounding claim would appear.
fn render(pattern: &PatternDeclaration) -> String {
    let mut out = String::new();
    writeln!(out, "pattern {}", pattern.id.as_str()).expect("string write");
    writeln!(out, "  name                   {}", pattern.name).expect("string write");
    writeln!(out, "  shape                  {}", pattern.shape.as_str()).expect("string write");
    writeln!(out, "  scope                  {}", pattern.scope.as_str()).expect("string write");
    writeln!(
        out,
        "  scope_always_closes    {}",
        pattern.scope.always_closes()
    )
    .expect("string write");
    writeln!(
        out,
        "  retirement             {}",
        pattern.retirement.as_str()
    )
    .expect("string write");
    writeln!(out, "  formula                {}", pattern.formula).expect("string write");
    writeln!(out, "  summary                {}", pattern.summary).expect("string write");
    writeln!(out, "  scope_note             {}", pattern.scope_note).expect("string write");
    writeln!(
        out,
        "  tier                   {}",
        pattern.cost.tier.as_str()
    )
    .expect("string write");
    writeln!(
        out,
        "  fact_requests/verdict  {}",
        pattern.cost.expected_fact_requests_per_verdict
    )
    .expect("string write");
    writeln!(out, "  grade                  {}", pattern.grade.as_str()).expect("string write");
    writeln!(
        out,
        "  width                  event_kinds={} attribute_keys={} total={} measured={}",
        pattern.width.budget.event_kinds,
        pattern.width.budget.attribute_keys,
        pattern.width.budget.event_kinds + pattern.width.budget.attribute_keys,
        pattern.width.measured
    )
    .expect("string write");
    writeln!(out, "  width_note             {}", pattern.width.note).expect("string write");
    for parameter in pattern.parameters {
        writeln!(
            out,
            "  parameter              {} : {}",
            parameter.name,
            parameter.kind.as_str()
        )
        .expect("string write");
        writeln!(out, "    description          {}", parameter.description).expect("string write");
    }
    for requirement in pattern.evidence {
        let capabilities: Vec<String> = requirement
            .capabilities
            .iter()
            .map(|capability| format!("{capability:?}"))
            .collect();
        writeln!(
            out,
            "  evidence               {} : {:?} [{}]",
            requirement.role,
            requirement.minimum_tier,
            capabilities.join(", ")
        )
        .expect("string write");
        writeln!(out, "    rationale            {}", requirement.rationale).expect("string write");
    }
    for reference in pattern.grounding {
        writeln!(out, "  grounding              {}", reference.document).expect("string write");
        writeln!(out, "    anchor               {}", reference.anchor).expect("string write");
        writeln!(out, "    supports             {}", reference.supports).expect("string write");
    }
    out
}

/// Renders the whole catalogue, header included.
fn render_catalogue() -> String {
    let mut out = String::new();
    out.push_str(
        "# Sentinel Pattern catalogue, rendered.\n\
         #\n\
         # Generated by compile/tests/catalogue_snapshot.rs. Issue #14 chose Rust data over a\n\
         # .proto for the catalogue (ADR-0024's canonical list does not include Patterns, and\n\
         # `just gen-probes` is a networked boundary the gates never invoke), so no `buf\n\
         # breaking` run guards this grammar. This file is the substitute: every change to a\n\
         # Pattern's parameters, evidence requirements, budget, cost class, grade or citations\n\
         # shows up here as a reviewed diff.\n\
         #\n\
         # If this file and the catalogue disagree, decide which one is wrong before updating\n\
         # either.\n\
         \n",
    );
    for pattern in catalogue() {
        out.push_str(&render(pattern));
    }
    out
}

/// The catalogue matches its golden snapshot.
#[test]
fn catalogue_matches_golden_snapshot() {
    let path = repository_root()
        .join("compile")
        .join("tests")
        .join("golden")
        .join("catalogue.txt");
    let actual = render_catalogue();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap_or_else(|error| {
            panic!(
                "failed to write golden snapshot {}: {error}",
                path.display()
            )
        });
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("golden snapshot {} is readable: {error}", path.display()));
    assert_eq!(
        actual,
        expected,
        "the catalogue no longer matches its golden snapshot.\n\
         If the change is intended, run with UPDATE_GOLDEN=1 and review the diff in {}.\n\
         --- actual ---\n{actual}",
        path.display()
    );
}

// ---------------------------------------------------------------------------
// O-07 traceability
// ---------------------------------------------------------------------------

/// **Every grounding reference resolves.**
///
/// For each `GroundingRef`, the named document exists on disk and the named
/// anchor occurs verbatim inside it. This is what makes O-07's traceability
/// mechanical rather than prose, and it is what makes a fabricated quotation a
/// build failure instead of something a reader has to catch.
///
/// It matters here specifically. The corpus's provenance ledger marks **only
/// S1** with "(expert-authored, no partner trace)"; S2 through S5 read a bare
/// "internal-scenario". A `GroundingRef` quoting the long string for S2-S5 would
/// be a fabricated quotation inside the very structure whose purpose is
/// traceability -- and this test would catch it.
#[test]
fn every_grounding_reference_resolves() {
    let root = repository_root();
    let mut checked = 0_usize;
    for pattern in catalogue() {
        for reference in pattern.grounding {
            let path = root.join(reference.document);
            let contents = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "{}: grounding document {} is unreadable: {error}",
                    pattern.id,
                    path.display()
                )
            });
            assert!(
                contents.contains(reference.anchor),
                "{}: anchor is not in {} -- the citation is stale or fabricated:\n  {}",
                pattern.id,
                reference.document,
                reference.anchor
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no grounding references were checked");
}

/// The catalogue's shape names are exactly the dashboard's `PatternShape` union.
///
/// The adversarial review found that `dashboard/src/data/mock.ts`'s `Pattern`
/// has five fields, not four -- the missed one, `id: PatternShape`, being
/// precisely the field #35 needs to wire the real catalogue to the existing
/// shell. This keeps the two vocabularies from drifting apart silently.
///
/// Note what it does *not* assert: that the catalogue's own `PatternId` values
/// appear there. They do not, and should not -- `PatternId` is a catalogue
/// identity and `PatternShape` is a temporal shape, and more than one Pattern
/// can share a shape. #35 needs both.
#[test]
fn shape_names_match_the_dashboard_pattern_shape_union() {
    let path = repository_root()
        .join("dashboard")
        .join("src")
        .join("data")
        .join("mock.ts");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        panic!("dashboard mock {} is readable", path.display());
    };
    for shape in TemporalShape::ALL {
        assert!(
            contents.contains(&format!("\"{}\"", shape.as_str())),
            "shape {} has no counterpart in the dashboard's PatternShape union",
            shape.as_str()
        );
    }
}
