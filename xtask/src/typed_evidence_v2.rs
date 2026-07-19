use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 2;
const SOURCE_TOP_K: usize = 5;
const MAX_CASES: usize = 256;
const MAX_SOURCES: usize = 16;
const MAX_EVIDENCE: usize = 128;
const MAX_ATOMS: usize = 32;
const MAX_QUALIFIERS: usize = 16;
const MAX_PERMUTATIONS: u8 = 64;
const BASIS_POINTS: u128 = 10_000;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ObjectType {
    Amount,
    Code,
    Condition,
    Date,
    Instruction,
    Other,
    Person,
    Procedure,
    Role,
    Statement,
    Status,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Polarity {
    Negative,
    Positive,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Lifecycle {
    Conditional,
    Current,
    Planned,
    Retracted,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provenance {
    Attributed,
    Direct,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualifierName {
    AccessScope,
    Comparator,
    EventKind,
    ReportingSource,
    TimeScope,
    Unit,
    Version,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct Qualifier {
    pub(crate) name: QualifierName,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claim {
    pub(crate) subject: String,
    pub(crate) relation: String,
    pub(crate) object_type: ObjectType,
    pub(crate) object_value: String,
    pub(crate) qualifiers: Vec<Qualifier>,
    pub(crate) polarity: Polarity,
    pub(crate) lifecycles: Vec<Lifecycle>,
    pub(crate) provenance: Provenance,
    pub(crate) support_quote: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct Need {
    pub(crate) subject: String,
    pub(crate) relation: String,
    pub(crate) requested_object_types: Vec<ObjectType>,
    pub(crate) required_qualifiers: Vec<Qualifier>,
    pub(crate) allowed_polarities: Vec<Polarity>,
    pub(crate) required_lifecycles: Vec<Lifecycle>,
    pub(crate) allowed_provenances: Vec<Provenance>,
    pub(crate) question_quote: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceAnnotation {
    pub(crate) schema_version: u32,
    pub(crate) fact_id: String,
    pub(crate) claims: Vec<Claim>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuestionAnnotation {
    pub(crate) schema_version: u32,
    pub(crate) case_id: String,
    pub(crate) needs: Vec<Need>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControlSpec {
    pub(crate) permutation_count: u8,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateSpec {
    pub(crate) min_recall_bps: u16,
    pub(crate) min_split_recall_bps: u16,
    pub(crate) min_exact_case_rate_bps: u16,
    pub(crate) max_unexpected_facts: usize,
    pub(crate) max_forbidden_facts: usize,
    pub(crate) min_exact_gain_over_structure_bps: u16,
    pub(crate) min_exact_gain_over_permutations_bps: u16,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvaluationSplit {
    Regression,
    Calibration,
    Holdout,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaseTag {
    Compound,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) source_id: String,
    pub(crate) ranked_evidence: Vec<SourceAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Case {
    pub(crate) case_id: String,
    pub(crate) split: EvaluationSplit,
    pub(crate) tags: Vec<CaseTag>,
    pub(crate) question: QuestionAnnotation,
    pub(crate) sources: Vec<Source>,
    pub(crate) expected_groups: Vec<Vec<String>>,
    pub(crate) allowed_support_fact_ids: Vec<String>,
    pub(crate) forbidden_fact_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fixture {
    pub(crate) cases: Vec<Case>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rate {
    pub(crate) numerator: usize,
    pub(crate) denominator: usize,
}

impl Rate {
    fn new(numerator: usize, denominator: usize) -> Self {
        if denominator == 0 {
            Self {
                numerator: 1,
                denominator: 1,
            }
        } else {
            Self {
                numerator,
                denominator,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SplitRate {
    pub(crate) split: EvaluationSplit,
    pub(crate) recall: Rate,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseOutcome {
    pub(crate) case_id: String,
    pub(crate) selected_facts: Vec<SelectedFact>,
    pub(crate) abstained: bool,
    pub(crate) exact: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedFact {
    pub(crate) source_id: String,
    pub(crate) fact_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArmMetrics {
    pub(crate) recall: Rate,
    pub(crate) precision: Rate,
    pub(crate) exact_case_rate: Rate,
    pub(crate) split_recall: Vec<SplitRate>,
    pub(crate) unexpected_fact_count: usize,
    pub(crate) forbidden_fact_count: usize,
    pub(crate) duplicate_error_count: usize,
    pub(crate) compound_partial_case_count: usize,
    pub(crate) conflict_missing_group_case_count: usize,
    pub(crate) abstained_case_count: usize,
    pub(crate) outcomes: Vec<CaseOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateFailure {
    Recall,
    SplitRecall,
    ExactCaseRate,
    UnexpectedFacts,
    ForbiddenFacts,
    DuplicateFacts,
    CompoundCoverage,
    ConflictCoverage,
    StructureControlGain,
    PermutationControlGain,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateReport {
    pub(crate) passed: bool,
    pub(crate) failures: Vec<GateFailure>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CeilingReport {
    pub(crate) treatment: ArmMetrics,
    pub(crate) structure_only_sham: ArmMetrics,
    pub(crate) claim_assignment_permutations: Vec<ArmMetrics>,
    pub(crate) gates: GateReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationError(String);

impl ValidationError {
    fn new(field: &str, reason: &str) -> Self {
        Self(format!("{field}: {reason}"))
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

#[derive(Clone, Copy)]
enum Arm {
    Treatment,
    StructureOnly,
    Permutation(usize),
}

#[derive(Clone, Copy)]
struct SelectedEvidence<'a> {
    source_id: &'a str,
    fact_id: &'a str,
    claims: &'a [Claim],
}

#[derive(Default)]
struct ArmTotals {
    covered_groups: usize,
    expected_groups: usize,
    correct_facts: usize,
    returned_facts: usize,
    exact_cases: usize,
    unexpected_facts: usize,
    forbidden_facts: usize,
    duplicate_errors: usize,
    compound_partial_cases: usize,
    conflict_missing_group_cases: usize,
}

pub(crate) fn claim_matches_need(claim: &Claim, need: &Need) -> bool {
    claim.subject == need.subject
        && claim.relation == need.relation
        && need
            .requested_object_types
            .binary_search(&claim.object_type)
            .is_ok()
        && need
            .required_qualifiers
            .iter()
            .all(|required| claim.qualifiers.binary_search(required).is_ok())
        && need
            .allowed_polarities
            .binary_search(&claim.polarity)
            .is_ok()
        && need
            .required_lifecycles
            .iter()
            .all(|required| claim.lifecycles.binary_search(required).is_ok())
        && need
            .allowed_provenances
            .binary_search(&claim.provenance)
            .is_ok()
}

pub(crate) fn validate_blind_source_annotation(
    annotation: &SourceAnnotation,
) -> Result<(), ValidationError> {
    validate_source(annotation)
}

pub(crate) fn validate_blind_question_annotation(
    annotation: &QuestionAnnotation,
) -> Result<(), ValidationError> {
    validate_question(annotation)
}

pub(crate) fn evaluate_ceiling(
    fixture: &Fixture,
    controls: ControlSpec,
    gates: GateSpec,
) -> Result<CeilingReport, ValidationError> {
    validate_fixture(fixture, controls, gates)?;
    let treatment = evaluate_arm(fixture, Arm::Treatment);
    let structure_only_sham = evaluate_arm(fixture, Arm::StructureOnly);
    let permutations = (0..usize::from(controls.permutation_count))
        .map(|index| evaluate_arm(fixture, Arm::Permutation(index)))
        .collect::<Vec<_>>();
    let gate_report = evaluate_gates(&treatment, &structure_only_sham, &permutations, gates);
    Ok(CeilingReport {
        treatment,
        structure_only_sham,
        claim_assignment_permutations: permutations,
        gates: gate_report,
    })
}

fn validate_fixture(
    fixture: &Fixture,
    controls: ControlSpec,
    gates: GateSpec,
) -> Result<(), ValidationError> {
    check_count("cases", fixture.cases.len(), 1, MAX_CASES)?;
    check_count(
        "permutation_count",
        usize::from(controls.permutation_count),
        1,
        usize::from(MAX_PERMUTATIONS),
    )?;
    for value in [
        gates.min_recall_bps,
        gates.min_split_recall_bps,
        gates.min_exact_case_rate_bps,
        gates.min_exact_gain_over_structure_bps,
        gates.min_exact_gain_over_permutations_bps,
    ] {
        if value > 10_000 {
            return Err(ValidationError::new("gates", "rate exceeds 10000 bps"));
        }
    }
    let mut case_ids = BTreeSet::new();
    for case in &fixture.cases {
        token(&case.case_id, "case_id")?;
        if !case_ids.insert(case.case_id.as_str()) {
            return Err(ValidationError::new("case_id", "duplicate"));
        }
        if case.question.case_id != case.case_id {
            return Err(ValidationError::new(
                "question.case_id",
                "does not match case",
            ));
        }
        sorted(&case.tags, "case.tags")?;
        validate_question(&case.question)?;
        validate_expectations(case)?;
        validate_sources(case)?;
    }
    Ok(())
}

fn validate_question(annotation: &QuestionAnnotation) -> Result<(), ValidationError> {
    if annotation.schema_version != SCHEMA_VERSION {
        return Err(ValidationError::new(
            "question.schema_version",
            "unsupported",
        ));
    }
    check_count("question.needs", annotation.needs.len(), 1, MAX_ATOMS)?;
    let mut needs = BTreeSet::new();
    for need in &annotation.needs {
        token(&need.subject, "need.subject")?;
        token(&need.relation, "need.relation")?;
        text(&need.question_quote, "need.question_quote")?;
        sorted_nonempty(&need.requested_object_types, "need.requested_object_types")?;
        sorted_nonempty(&need.allowed_polarities, "need.allowed_polarities")?;
        sorted_nonempty(&need.allowed_provenances, "need.allowed_provenances")?;
        sorted_nonempty(&need.required_lifecycles, "need.required_lifecycles")?;
        qualifiers(&need.required_qualifiers, "need.required_qualifiers")?;
        if !needs.insert(need) {
            return Err(ValidationError::new("question.needs", "duplicate"));
        }
    }
    Ok(())
}

fn validate_sources(case: &Case) -> Result<(), ValidationError> {
    check_count("sources", case.sources.len(), 1, MAX_SOURCES)?;
    let mut source_ids = BTreeSet::new();
    let mut facts = BTreeMap::<&str, &SourceAnnotation>::new();
    for source in &case.sources {
        token(&source.source_id, "source_id")?;
        if !source_ids.insert(source.source_id.as_str()) {
            return Err(ValidationError::new("source_id", "duplicate"));
        }
        check_count(
            "ranked_evidence",
            source.ranked_evidence.len(),
            0,
            MAX_EVIDENCE,
        )?;
        for annotation in &source.ranked_evidence {
            validate_source(annotation)?;
            if let Some(previous) = facts.insert(&annotation.fact_id, annotation)
                && previous != annotation
            {
                return Err(ValidationError::new("fact_id", "conflicting duplicate"));
            }
        }
    }
    Ok(())
}

fn validate_source(annotation: &SourceAnnotation) -> Result<(), ValidationError> {
    if annotation.schema_version != SCHEMA_VERSION {
        return Err(ValidationError::new("source.schema_version", "unsupported"));
    }
    token(&annotation.fact_id, "fact_id")?;
    check_count("claims", annotation.claims.len(), 1, MAX_ATOMS)?;
    let mut claims = BTreeSet::new();
    for claim in &annotation.claims {
        token(&claim.subject, "claim.subject")?;
        token(&claim.relation, "claim.relation")?;
        slug_text(&claim.object_value, "claim.object_value")?;
        text(&claim.support_quote, "claim.support_quote")?;
        sorted_nonempty(&claim.lifecycles, "claim.lifecycles")?;
        qualifiers(&claim.qualifiers, "claim.qualifiers")?;
        if !claims.insert(claim) {
            return Err(ValidationError::new("claims", "duplicate"));
        }
    }
    Ok(())
}

fn validate_expectations(case: &Case) -> Result<(), ValidationError> {
    let mut expected = BTreeSet::new();
    for group in &case.expected_groups {
        check_count("expected_group", group.len(), 1, MAX_EVIDENCE)?;
        let group_set = id_set(group, "expected_group")?;
        expected.extend(group_set);
    }
    let allowed = id_set(&case.allowed_support_fact_ids, "allowed_support")?;
    let forbidden = id_set(&case.forbidden_fact_ids, "forbidden")?;
    if expected
        .iter()
        .any(|id| allowed.contains(id) || forbidden.contains(id))
        || allowed.iter().any(|id| forbidden.contains(id))
    {
        return Err(ValidationError::new("expectations", "sets overlap"));
    }
    Ok(())
}

fn id_set<'a>(ids: &'a [String], field: &str) -> Result<BTreeSet<&'a str>, ValidationError> {
    let mut set = BTreeSet::new();
    for id in ids {
        token(id, field)?;
        if !set.insert(id.as_str()) {
            return Err(ValidationError::new(field, "duplicate"));
        }
    }
    Ok(set)
}

fn check_count(
    field: &str,
    value: usize,
    minimum: usize,
    maximum: usize,
) -> Result<(), ValidationError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(ValidationError::new(field, "count outside supported range"));
    }
    Ok(())
}

fn sorted<T: Ord>(values: &[T], field: &str) -> Result<(), ValidationError> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ValidationError::new(field, "must be unique and sorted"));
    }
    Ok(())
}

fn sorted_nonempty<T: Ord>(values: &[T], field: &str) -> Result<(), ValidationError> {
    check_count(field, values.len(), 1, MAX_ATOMS)?;
    sorted(values, field)
}

fn qualifiers(values: &[Qualifier], field: &str) -> Result<(), ValidationError> {
    check_count(field, values.len(), 0, MAX_QUALIFIERS)?;
    for qualifier in values {
        slug_text(&qualifier.value, field)?;
    }
    sorted(values, field)
}

fn token(value: &str, field: &str) -> Result<(), ValidationError> {
    slug(value, field, 64)
}

fn slug_text(value: &str, field: &str) -> Result<(), ValidationError> {
    slug(value, field, 512)
}

fn slug(value: &str, field: &str, maximum: usize) -> Result<(), ValidationError> {
    let mut segments = value.split('_');
    let Some(first) = segments.next() else {
        return Err(ValidationError::new(field, "not a canonical token"));
    };
    let valid_first = first
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase());
    let valid_segments = std::iter::once(first).chain(segments).all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    });
    if !valid_first || !valid_segments || value.len() > maximum {
        return Err(ValidationError::new(field, "not a canonical token"));
    }
    Ok(())
}

fn text(value: &str, field: &str) -> Result<(), ValidationError> {
    if value.trim() != value || value.is_empty() || value.len() > 512 || value.contains('\0') {
        return Err(ValidationError::new(field, "invalid text"));
    }
    Ok(())
}

fn evaluate_arm(fixture: &Fixture, arm: Arm) -> ArmMetrics {
    let mut outcomes = Vec::with_capacity(fixture.cases.len());
    let mut totals = ArmTotals::default();
    let mut split_counts = BTreeMap::<EvaluationSplit, (usize, usize)>::new();
    for case in &fixture.cases {
        let selected = select_case(case, arm);
        let ids = selected
            .iter()
            .map(|item| item.fact_id)
            .collect::<BTreeSet<_>>();
        let expected = case
            .expected_groups
            .iter()
            .flat_map(|group| group.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let allowed = case
            .allowed_support_fact_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let forbidden_set = case
            .forbidden_fact_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let covered = case
            .expected_groups
            .iter()
            .filter(|group| group.iter().any(|id| ids.contains(id.as_str())))
            .count();
        let unexpected = ids
            .iter()
            .filter(|id| !expected.contains(**id) && !allowed.contains(**id))
            .count();
        let forbidden = ids.iter().filter(|id| forbidden_set.contains(**id)).count();
        let duplicate_facts = selected.len().saturating_sub(ids.len());
        let duplicate_groups = case
            .expected_groups
            .iter()
            .filter(|group| group.iter().filter(|id| ids.contains(id.as_str())).count() > 1)
            .count();
        let duplicate_errors = duplicate_facts.saturating_add(duplicate_groups);
        let correct = ids
            .iter()
            .filter(|id| expected.contains(**id) || allowed.contains(**id))
            .count();
        let compound_partial = case.tags.contains(&CaseTag::Compound)
            && covered > 0
            && covered < case.expected_groups.len();
        let conflict_missing =
            case.tags.contains(&CaseTag::Conflict) && covered < case.expected_groups.len();
        let exact = if case.expected_groups.is_empty() {
            selected.is_empty()
        } else {
            covered == case.expected_groups.len()
                && unexpected == 0
                && forbidden == 0
                && duplicate_errors == 0
        };
        totals.covered_groups += covered;
        totals.expected_groups += case.expected_groups.len();
        totals.correct_facts += correct;
        totals.returned_facts += ids.len();
        totals.exact_cases += usize::from(exact);
        totals.unexpected_facts += unexpected;
        totals.forbidden_facts += forbidden;
        totals.duplicate_errors += duplicate_errors;
        totals.compound_partial_cases += usize::from(compound_partial);
        totals.conflict_missing_group_cases += usize::from(conflict_missing);
        let split = split_counts.entry(case.split).or_default();
        split.0 += covered;
        split.1 += case.expected_groups.len();
        outcomes.push(CaseOutcome {
            case_id: case.case_id.to_owned(),
            selected_facts: selected
                .iter()
                .map(|item| SelectedFact {
                    source_id: item.source_id.to_owned(),
                    fact_id: item.fact_id.to_owned(),
                })
                .collect(),
            abstained: selected.is_empty(),
            exact,
        });
    }
    ArmMetrics {
        recall: Rate::new(totals.covered_groups, totals.expected_groups),
        precision: Rate::new(totals.correct_facts, totals.returned_facts),
        exact_case_rate: Rate::new(totals.exact_cases, fixture.cases.len()),
        split_recall: split_counts
            .into_iter()
            .map(|(split, counts)| SplitRate {
                split,
                recall: Rate::new(counts.0, counts.1),
            })
            .collect(),
        unexpected_fact_count: totals.unexpected_facts,
        forbidden_fact_count: totals.forbidden_facts,
        duplicate_error_count: totals.duplicate_errors,
        compound_partial_case_count: totals.compound_partial_cases,
        conflict_missing_group_case_count: totals.conflict_missing_group_cases,
        abstained_case_count: outcomes.iter().filter(|outcome| outcome.abstained).count(),
        outcomes,
    }
}

fn select_case<'a>(case: &'a Case, arm: Arm) -> Vec<SelectedEvidence<'a>> {
    let mut selected = Vec::new();
    for source in &case.sources {
        let pool = unique_annotations(source);
        let indexes = pool
            .iter()
            .enumerate()
            .map(|(index, annotation)| (annotation.fact_id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let mut source_fact_ids = BTreeSet::new();
        let mut source_edges = BTreeSet::new();
        let mut source_count = 0usize;
        for annotation in &source.ranked_evidence {
            let fact_id = annotation.fact_id.as_str();
            if !source_fact_ids.insert(fact_id) {
                continue;
            }
            let claims = assigned_claims(annotation, arm, &pool, &indexes);
            let matches = match arm {
                Arm::StructureOnly => !claims.is_empty(),
                _ => {
                    let mut edges = claims
                        .iter()
                        .flat_map(|claim| {
                            case.question.needs.iter().enumerate().filter_map(
                                move |(need_index, need)| {
                                    claim_matches_need(claim, need)
                                        .then_some((need_index, claim.object_value.as_str()))
                                },
                            )
                        })
                        .collect::<BTreeSet<_>>();
                    edges.retain(|edge| !source_edges.contains(edge));
                    let has_new_edge = !edges.is_empty();
                    source_edges.extend(edges);
                    has_new_edge
                }
            };
            if matches {
                source_count += 1;
                selected.push(SelectedEvidence {
                    source_id: &source.source_id,
                    fact_id,
                    claims,
                });
                if source_count == SOURCE_TOP_K {
                    break;
                }
            }
        }
    }
    let complete = match arm {
        Arm::StructureOnly => {
            selected.iter().map(|item| item.claims.len()).sum::<usize>()
                >= case.question.needs.len()
        }
        _ => case.question.needs.iter().all(|need| {
            selected
                .iter()
                .flat_map(|item| item.claims)
                .any(|claim| claim_matches_need(claim, need))
        }),
    };
    if complete { selected } else { Vec::new() }
}

fn unique_annotations(source: &Source) -> Vec<&SourceAnnotation> {
    let mut seen = BTreeSet::new();
    source
        .ranked_evidence
        .iter()
        .filter(|annotation| seen.insert(annotation.fact_id.as_str()))
        .collect()
}

fn assigned_claims<'a>(
    annotation: &'a SourceAnnotation,
    arm: Arm,
    pool: &[&'a SourceAnnotation],
    indexes: &BTreeMap<&str, usize>,
) -> &'a [Claim] {
    let Arm::Permutation(permutation) = arm else {
        return &annotation.claims;
    };
    if pool.len() < 2 {
        return &[];
    }
    let Some(index) = indexes.get(annotation.fact_id.as_str()).copied() else {
        return &[];
    };
    let shift = 1 + permutation % (pool.len() - 1);
    &pool[(index + shift) % pool.len()].claims
}

fn evaluate_gates(
    treatment: &ArmMetrics,
    structure: &ArmMetrics,
    permutations: &[ArmMetrics],
    gates: GateSpec,
) -> GateReport {
    let mut failures = Vec::new();
    if !meets(treatment.recall, gates.min_recall_bps) {
        failures.push(GateFailure::Recall);
    }
    if treatment
        .split_recall
        .iter()
        .any(|split| !meets(split.recall, gates.min_split_recall_bps))
    {
        failures.push(GateFailure::SplitRecall);
    }
    if !meets(treatment.exact_case_rate, gates.min_exact_case_rate_bps) {
        failures.push(GateFailure::ExactCaseRate);
    }
    if treatment.unexpected_fact_count > gates.max_unexpected_facts {
        failures.push(GateFailure::UnexpectedFacts);
    }
    if treatment.forbidden_fact_count > gates.max_forbidden_facts {
        failures.push(GateFailure::ForbiddenFacts);
    }
    if treatment.duplicate_error_count > 0 {
        failures.push(GateFailure::DuplicateFacts);
    }
    if treatment.compound_partial_case_count > 0 {
        failures.push(GateFailure::CompoundCoverage);
    }
    if treatment.conflict_missing_group_case_count > 0 {
        failures.push(GateFailure::ConflictCoverage);
    }
    if !has_gain(
        treatment.exact_case_rate,
        structure.exact_case_rate,
        gates.min_exact_gain_over_structure_bps,
    ) {
        failures.push(GateFailure::StructureControlGain);
    }
    let best = permutations
        .iter()
        .map(|metrics| metrics.exact_case_rate)
        .max_by(compare_rates)
        .unwrap_or(Rate::new(0, 1));
    if !has_gain(
        treatment.exact_case_rate,
        best,
        gates.min_exact_gain_over_permutations_bps,
    ) {
        failures.push(GateFailure::PermutationControlGain);
    }
    GateReport {
        passed: failures.is_empty(),
        failures,
    }
}

fn meets(rate: Rate, required_bps: u16) -> bool {
    rate.numerator as u128 * BASIS_POINTS >= u128::from(required_bps) * rate.denominator as u128
}

fn compare_rates(left: &Rate, right: &Rate) -> std::cmp::Ordering {
    (left.numerator as u128 * right.denominator as u128)
        .cmp(&(right.numerator as u128 * left.denominator as u128))
}

fn has_gain(treatment: Rate, control: Rate, required_bps: u16) -> bool {
    let treatment_scaled = treatment.numerator as u128 * control.denominator as u128 * BASIS_POINTS;
    let control_scaled = control.numerator as u128 * treatment.denominator as u128 * BASIS_POINTS;
    let required_scaled =
        u128::from(required_bps) * treatment.denominator as u128 * control.denominator as u128;
    treatment_scaled >= control_scaled + required_scaled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(relation: &str, value: &str) -> Claim {
        Claim {
            subject: "atlas".to_owned(),
            relation: relation.to_owned(),
            object_type: ObjectType::Statement,
            object_value: value.to_owned(),
            qualifiers: Vec::new(),
            polarity: Polarity::Positive,
            lifecycles: vec![Lifecycle::Current],
            provenance: Provenance::Direct,
            support_quote: "synthetic support".to_owned(),
        }
    }

    fn need(relation: &str) -> Need {
        Need {
            subject: "atlas".to_owned(),
            relation: relation.to_owned(),
            requested_object_types: vec![ObjectType::Statement],
            required_qualifiers: Vec::new(),
            allowed_polarities: vec![Polarity::Positive],
            required_lifecycles: vec![Lifecycle::Current],
            allowed_provenances: vec![Provenance::Direct],
            question_quote: "synthetic question".to_owned(),
        }
    }

    fn annotation(id: &str, claims: Vec<Claim>) -> SourceAnnotation {
        SourceAnnotation {
            schema_version: SCHEMA_VERSION,
            fact_id: id.to_owned(),
            claims,
        }
    }

    fn gates() -> GateSpec {
        GateSpec {
            min_recall_bps: 10_000,
            min_split_recall_bps: 10_000,
            min_exact_case_rate_bps: 10_000,
            max_unexpected_facts: 0,
            max_forbidden_facts: 0,
            min_exact_gain_over_structure_bps: 10_000,
            min_exact_gain_over_permutations_bps: 10_000,
        }
    }

    fn open_gates() -> GateSpec {
        GateSpec {
            min_recall_bps: 0,
            min_split_recall_bps: 0,
            min_exact_case_rate_bps: 0,
            max_unexpected_facts: usize::MAX,
            max_forbidden_facts: usize::MAX,
            min_exact_gain_over_structure_bps: 0,
            min_exact_gain_over_permutations_bps: 0,
        }
    }

    fn tagged_partial_fixture(tag: CaseTag) -> Fixture {
        Fixture {
            cases: vec![Case {
                case_id: "partial_case".to_owned(),
                split: EvaluationSplit::Holdout,
                tags: vec![tag],
                question: QuestionAnnotation {
                    schema_version: SCHEMA_VERSION,
                    case_id: "partial_case".to_owned(),
                    needs: vec![need("owner")],
                },
                sources: vec![Source {
                    source_id: "origin".to_owned(),
                    ranked_evidence: vec![annotation("owner_fact", vec![claim("owner", "camila")])],
                }],
                expected_groups: vec![
                    vec!["owner_fact".to_owned()],
                    vec!["missing_fact".to_owned()],
                ],
                allowed_support_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
            }],
        }
    }

    #[test]
    fn serde_rejects_unknown_fields() {
        let json = r#"{"schema_version":2,"fact_id":"fact_a","claims":[],"extra":true}"#;
        assert!(serde_json::from_str::<SourceAnnotation>(json).is_err());
    }

    #[test]
    fn matcher_requires_every_requested_slot() {
        let matching = claim("owner", "camila");
        let mut withdrawn = matching.clone();
        withdrawn.lifecycles = vec![Lifecycle::Retracted];
        assert!(claim_matches_need(&matching, &need("owner")));
        assert!(!claim_matches_need(&withdrawn, &need("owner")));
    }

    #[test]
    fn validation_rejects_noncanonical_lifecycle_order() {
        let mut invalid = claim("owner", "camila");
        invalid.lifecycles = vec![Lifecycle::Retracted, Lifecycle::Current];
        let result = validate_source(&annotation("owner_fact", vec![invalid]));
        assert!(result.is_err());
    }

    #[test]
    fn validation_rejects_resolved_source_without_claims() {
        assert!(validate_source(&annotation("empty_fact", Vec::new())).is_err());
    }

    #[test]
    fn validation_rejects_resolved_question_without_needs() {
        let annotation = QuestionAnnotation {
            schema_version: SCHEMA_VERSION,
            case_id: "empty_question".to_owned(),
            needs: Vec::new(),
        };

        assert!(validate_question(&annotation).is_err());
    }

    #[test]
    fn validation_rejects_empty_claim_lifecycle() {
        let mut invalid = claim("owner", "camila");
        invalid.lifecycles.clear();

        assert!(validate_source(&annotation("owner_fact", vec![invalid])).is_err());
    }

    #[test]
    fn validation_rejects_empty_need_lifecycle() {
        let mut invalid = need("owner");
        invalid.required_lifecycles.clear();
        let annotation = QuestionAnnotation {
            schema_version: SCHEMA_VERSION,
            case_id: "owner_case".to_owned(),
            needs: vec![invalid],
        };

        assert!(validate_question(&annotation).is_err());
    }

    #[test]
    fn validation_rejects_slug_with_empty_segment() {
        assert!(token("atlas__owner", "test").is_err());
    }

    #[test]
    fn validation_rejects_slug_with_trailing_separator() {
        assert!(token("atlas_owner_", "test").is_err());
    }

    #[test]
    fn validation_rejects_noncanonical_object_value() {
        let invalid = claim("owner", "two words");

        assert!(validate_source(&annotation("owner_fact", vec![invalid])).is_err());
    }

    #[test]
    fn validation_rejects_noncanonical_qualifier_value() {
        let mut invalid = claim("owner", "camila");
        invalid.qualifiers.push(Qualifier {
            name: QualifierName::TimeScope,
            value: "next quarter".to_owned(),
        });

        assert!(validate_source(&annotation("owner_fact", vec![invalid])).is_err());
    }

    #[test]
    fn deserialization_rejects_unknown_qualifier_name() {
        let json = r#"{"name":"invented_scope","value":"current"}"#;

        assert!(serde_json::from_str::<Qualifier>(json).is_err());
    }

    #[test]
    fn gateway_abstains_when_a_conjunct_is_missing() {
        let owner = annotation("owner_fact", vec![claim("owner", "camila")]);
        let question = QuestionAnnotation {
            schema_version: 2,
            case_id: "compound_case".to_owned(),
            needs: vec![need("owner"), need("target_date")],
        };
        let case = Case {
            case_id: "compound_case".to_owned(),
            split: EvaluationSplit::Holdout,
            tags: vec![CaseTag::Compound],
            question,
            sources: vec![Source {
                source_id: "origin".to_owned(),
                ranked_evidence: vec![owner],
            }],
            expected_groups: vec![vec!["owner_fact".to_owned()], vec!["date_fact".to_owned()]],
            allowed_support_fact_ids: Vec::new(),
            forbidden_fact_ids: Vec::new(),
        };
        assert!(select_case(&case, Arm::Treatment).is_empty());
    }

    #[test]
    fn no_answer_case_is_exact_only_when_it_abstains() {
        let owner = annotation("owner_fact", vec![claim("owner", "camila")]);
        let question = QuestionAnnotation {
            schema_version: 2,
            case_id: "no_answer".to_owned(),
            needs: vec![need("owner")],
        };
        let fixture = Fixture {
            cases: vec![Case {
                case_id: "no_answer".to_owned(),
                split: EvaluationSplit::Calibration,
                tags: Vec::new(),
                question,
                sources: vec![Source {
                    source_id: "origin".to_owned(),
                    ranked_evidence: vec![owner],
                }],
                expected_groups: Vec::new(),
                allowed_support_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
            }],
        };

        let metrics = evaluate_arm(&fixture, Arm::Treatment);

        assert!(!metrics.outcomes[0].exact);
    }

    #[test]
    fn returning_two_equivalent_answers_is_a_duplicate_error() {
        let first = annotation("owner_a", vec![claim("owner", "camila")]);
        let second = annotation("owner_b", vec![claim("owner", "diego")]);
        let question = QuestionAnnotation {
            schema_version: 2,
            case_id: "duplicate_group".to_owned(),
            needs: vec![need("owner")],
        };
        let fixture = Fixture {
            cases: vec![Case {
                case_id: "duplicate_group".to_owned(),
                split: EvaluationSplit::Holdout,
                tags: Vec::new(),
                question,
                sources: vec![Source {
                    source_id: "origin".to_owned(),
                    ranked_evidence: vec![first, second],
                }],
                expected_groups: vec![vec!["owner_a".to_owned(), "owner_b".to_owned()]],
                allowed_support_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
            }],
        };

        let metrics = evaluate_arm(&fixture, Arm::Treatment);

        assert_eq!(metrics.duplicate_error_count, 1);
        assert!(!metrics.outcomes[0].exact);
    }

    #[test]
    fn compound_partial_coverage_fails_its_explicit_gate() {
        let report = evaluate_ceiling(
            &tagged_partial_fixture(CaseTag::Compound),
            ControlSpec {
                permutation_count: 1,
            },
            open_gates(),
        )
        .expect("valid compound fixture");

        assert!(
            report
                .gates
                .failures
                .contains(&GateFailure::CompoundCoverage)
        );
    }

    #[test]
    fn conflict_missing_group_fails_its_explicit_gate() {
        let report = evaluate_ceiling(
            &tagged_partial_fixture(CaseTag::Conflict),
            ControlSpec {
                permutation_count: 1,
            },
            open_gates(),
        )
        .expect("valid conflict fixture");

        assert!(
            report
                .gates
                .failures
                .contains(&GateFailure::ConflictCoverage)
        );
    }

    #[test]
    fn source_top_five_deduplicates_match_edges_before_truncating() {
        let annotations = vec![
            annotation("owner_camila", vec![claim("owner", "camila")]),
            annotation("owner_duplicate", vec![claim("owner", "camila")]),
            annotation("owner_diego", vec![claim("owner", "diego")]),
            annotation("owner_lucia", vec![claim("owner", "lucia")]),
            annotation("owner_pedro", vec![claim("owner", "pedro")]),
            annotation("status_green", vec![claim("status", "green")]),
        ];
        let question = QuestionAnnotation {
            schema_version: 2,
            case_id: "top_five".to_owned(),
            needs: vec![need("owner"), need("status")],
        };
        let case = Case {
            case_id: "top_five".to_owned(),
            split: EvaluationSplit::Calibration,
            tags: Vec::new(),
            question,
            sources: vec![Source {
                source_id: "origin".to_owned(),
                ranked_evidence: annotations,
            }],
            expected_groups: vec![
                vec![
                    "owner_camila".to_owned(),
                    "owner_duplicate".to_owned(),
                    "owner_diego".to_owned(),
                    "owner_lucia".to_owned(),
                    "owner_pedro".to_owned(),
                ],
                vec!["status_green".to_owned()],
            ],
            allowed_support_fact_ids: Vec::new(),
            forbidden_fact_ids: Vec::new(),
        };
        let selected_ids = select_case(&case, Arm::Treatment)
            .into_iter()
            .map(|evidence| evidence.fact_id)
            .collect::<Vec<_>>();
        assert_eq!(
            selected_ids,
            vec![
                "owner_camila",
                "owner_diego",
                "owner_lucia",
                "owner_pedro",
                "status_green"
            ]
        );
    }

    #[test]
    fn treatment_beats_both_controls() {
        let noise = annotation("noise_fact", vec![claim("status", "green")]);
        let owner = annotation("owner_fact", vec![claim("owner", "camila")]);
        let question = QuestionAnnotation {
            schema_version: 2,
            case_id: "owner_case".to_owned(),
            needs: vec![need("owner")],
        };
        let fixture = Fixture {
            cases: vec![Case {
                case_id: "owner_case".to_owned(),
                split: EvaluationSplit::Regression,
                tags: Vec::new(),
                question,
                sources: vec![Source {
                    source_id: "origin".to_owned(),
                    ranked_evidence: vec![noise, owner],
                }],
                expected_groups: vec![vec!["owner_fact".to_owned()]],
                allowed_support_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
            }],
        };
        let report = evaluate_ceiling(
            &fixture,
            ControlSpec {
                permutation_count: 1,
            },
            gates(),
        )
        .expect("valid synthetic fixture");
        assert!(report.gates.passed);
        assert_eq!(
            report.treatment.outcomes[0]
                .selected_facts
                .iter()
                .map(|selected| selected.fact_id.as_str())
                .collect::<Vec<_>>(),
            ["owner_fact"]
        );
        assert!(!report.structure_only_sham.outcomes[0].exact);
        assert_eq!(
            report.claim_assignment_permutations[0].outcomes[0]
                .selected_facts
                .iter()
                .map(|selected| selected.fact_id.as_str())
                .collect::<Vec<_>>(),
            ["noise_fact"]
        );
    }
}
