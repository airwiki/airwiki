use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::workspace_root;

const CORPUS_DIRECTORY: &str = "fixtures/selector/answerability-v1";
const QUERY_FILE: &str = "queries.jsonl";
const PASSAGE_FILE: &str = "passages.jsonl";
const JUDGMENT_FILE: &str = "judgments.jsonl";
const JUDGMENTS_PER_QUERY: usize = 6;
const TRAIN_QUERY_COUNT: usize = 120;
const DEV_QUERY_COUNT: usize = 32;
const TRAIN_NO_ANSWER_COUNT: usize = 30;
const DEV_NO_ANSWER_COUNT: usize = 8;
const TRAIN_DIRECTION_COUNT: usize = 30;
const DEV_DIRECTION_COUNT: usize = 8;
const TRAIN_MULTI_ANSWER_COUNT: usize = 30;
const DEV_MULTI_ANSWER_COUNT: usize = 8;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_QUERY_BYTES: usize = 2 * 1024;
const MAX_PASSAGE_FIELD_CHARS: usize = 200;
const MAX_PASSAGE_TEXT_CHARS: usize = 1_200;
const MAX_REVIEW_REASON_CHARS: usize = 500;
const SEALED_FILES: [(&str, &str); 3] = [
    (
        QUERY_FILE,
        "41d6b1a2c093a920339081f4f2c616e81027e7f69409673f7c511167ecf61c4f",
    ),
    (
        PASSAGE_FILE,
        "3418cf2e5604894800da388ba6e41afc0e0f620c9f64173f4ac1f321b4559696",
    ),
    (
        JUDGMENT_FILE,
        "f1d66311a1b799452564a25407ae54980b89d86f8558c255e7be6b28347eee6e",
    ),
];

const PRODUCTION_REQUIREMENTS: CorpusRequirements = CorpusRequirements {
    train: SplitRequirements {
        query_count: TRAIN_QUERY_COUNT,
        no_answer_count: TRAIN_NO_ANSWER_COUNT,
        direction_count: TRAIN_DIRECTION_COUNT,
        minimum_multi_answer_count: TRAIN_MULTI_ANSWER_COUNT,
    },
    dev: SplitRequirements {
        query_count: DEV_QUERY_COUNT,
        no_answer_count: DEV_NO_ANSWER_COUNT,
        direction_count: DEV_DIRECTION_COUNT,
        minimum_multi_answer_count: DEV_MULTI_ANSWER_COUNT,
    },
};

const REQUIRED_TAGS: [TaxonomyTag; 17] = [
    TaxonomyTag::Direct,
    TaxonomyTag::Paraphrase,
    TaxonomyTag::Compound,
    TaxonomyTag::Absence,
    TaxonomyTag::Conflict,
    TaxonomyTag::Duplicate,
    TaxonomyTag::InjectionRequested,
    TaxonomyTag::InjectionUnrelated,
    TaxonomyTag::WrongEntity,
    TaxonomyTag::WrongRelation,
    TaxonomyTag::WrongDate,
    TaxonomyTag::WrongVersion,
    TaxonomyTag::WrongScope,
    TaxonomyTag::Negation,
    TaxonomyTag::Ambiguity,
    TaxonomyTag::MetadataOnly,
    TaxonomyTag::LongPassage,
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    query_id: String,
    text: String,
    language: Language,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Passage {
    passage_id: String,
    title: String,
    heading: String,
    text: String,
    language: Language,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Judgment {
    query_id: String,
    passage_id: String,
    split: Split,
    world_id: String,
    role: JudgmentRole,
    answer_group_id: Option<String>,
    disclosure: Disclosure,
    tags: Vec<TaxonomyTag>,
    negative_kind: Option<NegativeKind>,
    evidence_spans: Vec<EvidenceSpan>,
    review_reason: String,
    review_state: ReviewState,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Language {
    Es,
    En,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Split {
    Train,
    Dev,
}

impl Split {
    const fn world_prefix(self) -> &'static str {
        match self {
            Self::Train => "train_",
            Self::Dev => "dev_",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Dev => "dev",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JudgmentRole {
    Answer,
    Support,
    HardNegative,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Disclosure {
    Allowed,
    Forbidden,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReviewState {
    SyntheticDraft,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NegativeKind {
    SupportContext,
    WrongEntity,
    WrongRelation,
    WrongDate,
    WrongVersion,
    WrongScope,
    Negation,
    Ambiguity,
    MetadataOnly,
    UnrelatedInjection,
    Random,
}

impl NegativeKind {
    const fn is_hard(self) -> bool {
        !matches!(self, Self::SupportContext)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum TaxonomyTag {
    Direct,
    Paraphrase,
    Compound,
    Absence,
    Conflict,
    Duplicate,
    InjectionRequested,
    InjectionUnrelated,
    WrongEntity,
    WrongRelation,
    WrongDate,
    WrongVersion,
    WrongScope,
    Negation,
    Ambiguity,
    MetadataOnly,
    LongPassage,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceSpan {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LanguageDirection {
    EsEs,
    EnEn,
    EnEs,
    EsEn,
}

impl LanguageDirection {
    const ALL: [Self; 4] = [Self::EsEs, Self::EnEn, Self::EnEs, Self::EsEn];

    const fn new(query: Language, passage: Language) -> Self {
        match (query, passage) {
            (Language::Es, Language::Es) => Self::EsEs,
            (Language::En, Language::En) => Self::EnEn,
            (Language::En, Language::Es) => Self::EnEs,
            (Language::Es, Language::En) => Self::EsEn,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::EsEs => "es-es",
            Self::EnEn => "en-en",
            Self::EnEs => "en-es",
            Self::EsEn => "es-en",
        }
    }
}

#[derive(Debug, Default)]
struct SplitStats {
    query_count: usize,
    no_answer_count: usize,
    multi_answer_count: usize,
    language_directions: BTreeMap<LanguageDirection, usize>,
    tags: BTreeSet<TaxonomyTag>,
    disclosures: BTreeSet<Disclosure>,
    forbidden_answer_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct ValidationSummary {
    train_queries: usize,
    dev_queries: usize,
    judgments: usize,
}

#[derive(Debug, Clone, Copy)]
struct SplitRequirements {
    query_count: usize,
    no_answer_count: usize,
    direction_count: usize,
    minimum_multi_answer_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct CorpusRequirements {
    train: SplitRequirements,
    dev: SplitRequirements,
}

pub fn validate() -> Result<()> {
    let directory = workspace_root().join(CORPUS_DIRECTORY);
    validate_sealed_files(&directory)?;
    let summary = validate_directory(&directory)?;
    println!(
        "validated selector answerability corpus: {} train queries, {} dev queries, {} judgments",
        summary.train_queries, summary.dev_queries, summary.judgments
    );
    Ok(())
}

fn validate_sealed_files(directory: &Path) -> Result<()> {
    for (file_name, expected_sha256) in SEALED_FILES {
        let path = directory.join(file_name);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading sealed selector corpus file {}", path.display()))?;
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        ensure!(
            actual_sha256 == expected_sha256,
            "sealed selector corpus file `{file_name}` has SHA-256 {actual_sha256}, expected {expected_sha256}; create a new corpus version instead of changing sealed data"
        );
    }
    Ok(())
}

fn validate_directory(directory: &Path) -> Result<ValidationSummary> {
    validate_directory_with_requirements(directory, PRODUCTION_REQUIREMENTS)
}

fn validate_directory_with_requirements(
    directory: &Path,
    requirements: CorpusRequirements,
) -> Result<ValidationSummary> {
    let queries: Vec<Query> = load_jsonl(&directory.join(QUERY_FILE))?;
    let passages: Vec<Passage> = load_jsonl(&directory.join(PASSAGE_FILE))?;
    let judgments: Vec<Judgment> = load_jsonl(&directory.join(JUDGMENT_FILE))?;
    validate_corpus(&queries, &passages, &judgments, requirements)
}

fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line =
            line.with_context(|| format!("reading line {line_number} from {}", path.display()))?;
        ensure!(
            !line.trim().is_empty(),
            "{} contains a blank line at {line_number}",
            path.display()
        );
        let record = serde_json::from_str(&line)
            .with_context(|| format!("parsing line {line_number} from {}", path.display()))?;
        records.push(record);
    }
    ensure!(!records.is_empty(), "{} is empty", path.display());
    Ok(records)
}

fn validate_corpus(
    queries: &[Query],
    passages: &[Passage],
    judgments: &[Judgment],
    requirements: CorpusRequirements,
) -> Result<ValidationSummary> {
    let query_by_id = validate_queries(queries)?;
    let passage_by_id = validate_passages(passages)?;
    let judgments_by_query = validate_judgments(judgments, &query_by_id, &passage_by_id)?;
    ensure!(
        judgments_by_query.len() == queries.len(),
        "every selector query must be referenced by judgments"
    );

    let mut passage_reference_counts = HashMap::<&str, usize>::new();
    let mut world_ids = HashSet::<&str>::new();
    let mut normalized_queries = HashMap::<String, Split>::new();
    let mut normalized_passages = HashMap::<String, Split>::new();
    let mut stats = BTreeMap::from([
        (Split::Train, SplitStats::default()),
        (Split::Dev, SplitStats::default()),
    ]);

    for query in queries {
        let query_judgments = judgments_by_query
            .get(query.query_id.as_str())
            .with_context(|| format!("query `{}` has no judgments", query.query_id))?;
        validate_query_pool(
            query,
            query_judgments,
            &passage_by_id,
            &mut passage_reference_counts,
            &mut world_ids,
            &mut normalized_queries,
            &mut normalized_passages,
            &mut stats,
        )?;
    }

    ensure!(
        passage_reference_counts.len() == passages.len()
            && passage_reference_counts.values().all(|count| *count == 1),
        "every selector passage must be referenced exactly once"
    );
    validate_cross_split_template_isolation(judgments, &passage_by_id)?;
    validate_split_stats(&stats, requirements)?;

    Ok(ValidationSummary {
        train_queries: stats
            .get(&Split::Train)
            .context("train selector statistics are unavailable")?
            .query_count,
        dev_queries: stats
            .get(&Split::Dev)
            .context("dev selector statistics are unavailable")?
            .query_count,
        judgments: judgments.len(),
    })
}

fn validate_cross_split_template_isolation(
    judgments: &[Judgment],
    passages: &HashMap<&str, &Passage>,
) -> Result<()> {
    let mut passages_by_split_and_language =
        BTreeMap::<(Split, Language), Vec<(&str, TemplateShingles)>>::new();
    for judgment in judgments {
        let passage = passages
            .get(judgment.passage_id.as_str())
            .context("a validated passage is unavailable for template isolation")?;
        passages_by_split_and_language
            .entry((judgment.split, passage.language))
            .or_default()
            .push((
                judgment.passage_id.as_str(),
                five_token_shingles(&canonical_template_tokens(&passage.text)),
            ));
    }

    for language in [Language::Es, Language::En] {
        let train = passages_by_split_and_language
            .get(&(Split::Train, language))
            .context("train passages are unavailable for template isolation")?;
        let dev = passages_by_split_and_language
            .get(&(Split::Dev, language))
            .context("dev passages are unavailable for template isolation")?;
        for (train_id, train_shingles) in train {
            for (dev_id, dev_shingles) in dev {
                ensure!(
                    !templates_overlap(train_shingles, dev_shingles),
                    "selector template leaks across train passage `{train_id}` and dev passage `{dev_id}`"
                );
            }
        }
    }
    Ok(())
}

fn validate_queries(queries: &[Query]) -> Result<HashMap<&str, &Query>> {
    let mut by_id = HashMap::new();
    for query in queries {
        validate_identifier(&query.query_id, "query")?;
        validate_nonempty_bounded_bytes(&query.text, MAX_QUERY_BYTES, "query text")?;
        ensure!(
            by_id.insert(query.query_id.as_str(), query).is_none(),
            "duplicate selector query id `{}`",
            query.query_id
        );
    }
    Ok(by_id)
}

fn validate_passages(passages: &[Passage]) -> Result<HashMap<&str, &Passage>> {
    let mut by_id = HashMap::new();
    for passage in passages {
        validate_identifier(&passage.passage_id, "passage")?;
        validate_nonempty_bounded_chars(&passage.title, MAX_PASSAGE_FIELD_CHARS, "passage title")?;
        validate_nonempty_bounded_chars(
            &passage.heading,
            MAX_PASSAGE_FIELD_CHARS,
            "passage heading",
        )?;
        validate_nonempty_bounded_chars(&passage.text, MAX_PASSAGE_TEXT_CHARS, "passage text")?;
        ensure!(
            by_id.insert(passage.passage_id.as_str(), passage).is_none(),
            "duplicate selector passage id `{}`",
            passage.passage_id
        );
    }
    Ok(by_id)
}

fn validate_judgments<'a>(
    judgments: &'a [Judgment],
    queries: &HashMap<&str, &Query>,
    passages: &HashMap<&str, &Passage>,
) -> Result<HashMap<&'a str, Vec<&'a Judgment>>> {
    let mut by_query = HashMap::<&str, Vec<&Judgment>>::new();
    let mut query_passage_pairs = HashSet::new();
    for judgment in judgments {
        ensure!(
            queries.contains_key(judgment.query_id.as_str()),
            "judgment references unknown query `{}`",
            judgment.query_id
        );
        let passage = passages
            .get(judgment.passage_id.as_str())
            .with_context(|| {
                format!(
                    "judgment for query `{}` references unknown passage `{}`",
                    judgment.query_id, judgment.passage_id
                )
            })?;
        ensure!(
            query_passage_pairs.insert((judgment.query_id.as_str(), judgment.passage_id.as_str())),
            "query `{}` contains duplicate judgment for passage `{}`",
            judgment.query_id,
            judgment.passage_id
        );
        validate_judgment(judgment, passage)?;
        by_query
            .entry(judgment.query_id.as_str())
            .or_default()
            .push(judgment);
    }
    Ok(by_query)
}

fn validate_judgment(judgment: &Judgment, passage: &Passage) -> Result<()> {
    validate_identifier(&judgment.world_id, "world")?;
    ensure!(
        judgment.world_id.starts_with(judgment.split.world_prefix()),
        "world `{}` does not match split `{}`",
        judgment.world_id,
        judgment.split.label()
    );
    validate_nonempty_bounded_chars(
        &judgment.review_reason,
        MAX_REVIEW_REASON_CHARS,
        "review reason",
    )?;
    ensure!(
        judgment.review_state == ReviewState::SyntheticDraft,
        "selector judgments must remain synthetic drafts"
    );
    ensure!(
        !judgment.tags.is_empty(),
        "judgment for query `{}` has no taxonomy tags",
        judgment.query_id
    );
    let unique_tags = judgment.tags.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        unique_tags.len() == judgment.tags.len(),
        "judgment for query `{}` repeats a taxonomy tag",
        judgment.query_id
    );

    match judgment.role {
        JudgmentRole::Answer => {
            let group_id = judgment
                .answer_group_id
                .as_deref()
                .context("answer judgment is missing `answer_group_id`")?;
            validate_identifier(group_id, "answer group")?;
            ensure!(
                judgment.negative_kind.is_none(),
                "answer judgment must not define `negative_kind`"
            );
            ensure!(
                !judgment.evidence_spans.is_empty(),
                "answer judgment must contain evidence spans"
            );
            validate_evidence_spans(&judgment.evidence_spans, &passage.text)?;
        }
        JudgmentRole::Support => {
            ensure!(
                judgment.answer_group_id.is_none(),
                "support judgment must not define `answer_group_id`"
            );
            ensure!(
                judgment.evidence_spans.is_empty(),
                "support judgment must not contain evidence spans"
            );
            ensure!(
                judgment.negative_kind == Some(NegativeKind::SupportContext),
                "support judgment must use `support_context`"
            );
        }
        JudgmentRole::HardNegative => {
            ensure!(
                judgment.answer_group_id.is_none(),
                "hard-negative judgment must not define `answer_group_id`"
            );
            ensure!(
                judgment.evidence_spans.is_empty(),
                "hard-negative judgment must not contain evidence spans"
            );
            ensure!(
                judgment.negative_kind.is_some_and(NegativeKind::is_hard),
                "hard-negative judgment must define a hard `negative_kind`"
            );
        }
    }
    Ok(())
}

fn validate_evidence_spans(spans: &[EvidenceSpan], passage: &str) -> Result<()> {
    let mut unique_spans = BTreeSet::new();
    for span in spans {
        let start = usize::try_from(span.start).context("evidence span start is too large")?;
        let end = usize::try_from(span.end).context("evidence span end is too large")?;
        ensure!(start < end, "evidence span must be nonempty");
        ensure!(end <= passage.len(), "evidence span exceeds passage bytes");
        ensure!(
            passage.is_char_boundary(start) && passage.is_char_boundary(end),
            "evidence span must use UTF-8 byte boundaries"
        );
        ensure!(
            !passage[start..end].trim().is_empty(),
            "evidence span selects only whitespace"
        );
        ensure!(
            unique_spans.insert((start, end)),
            "answer judgment repeats an evidence span"
        );
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the explicit validation indexes keep corpus invariants visible"
)]
fn validate_query_pool<'a>(
    query: &Query,
    judgments: &[&'a Judgment],
    passages: &HashMap<&str, &'a Passage>,
    passage_reference_counts: &mut HashMap<&'a str, usize>,
    world_ids: &mut HashSet<&'a str>,
    normalized_queries: &mut HashMap<String, Split>,
    normalized_passages: &mut HashMap<String, Split>,
    stats: &mut BTreeMap<Split, SplitStats>,
) -> Result<()> {
    ensure!(
        judgments.len() == JUDGMENTS_PER_QUERY,
        "query `{}` must have exactly {JUDGMENTS_PER_QUERY} judgments",
        query.query_id
    );
    let first = judgments
        .first()
        .context("a selector query pool unexpectedly has no judgments")?;
    let split = first.split;
    let world_id = first.world_id.as_str();
    ensure!(
        judgments
            .iter()
            .all(|judgment| judgment.split == split && judgment.world_id == world_id),
        "all judgments for query `{}` must share one split and world",
        query.query_id
    );
    ensure!(
        world_ids.insert(world_id),
        "world `{world_id}` is assigned to more than one query"
    );
    validate_model_visible_text(
        &query.text,
        "query text",
        &[query.query_id.as_str(), world_id],
    )?;

    let normalized_query = normalize_for_leakage(&query.text);
    reject_cross_split_normalized_value(normalized_queries, normalized_query, split, "query text")?;

    let first_passage = passages
        .get(first.passage_id.as_str())
        .context("the first query passage is unavailable")?;
    let passage_language = first_passage.language;
    let title = first_passage.title.as_str();
    let heading = first_passage.heading.as_str();
    let mut answer_count = 0;
    let mut support_count = 0;
    let mut hard_negative_count = 0;
    for judgment in judgments {
        let passage = passages
            .get(judgment.passage_id.as_str())
            .context("a validated query passage is unavailable")?;
        ensure!(
            passage.language == passage_language,
            "all passages for query `{}` must share one language",
            query.query_id
        );
        ensure!(
            passage.title == title && passage.heading == heading,
            "all passages for query `{}` must share neutral title and heading metadata",
            query.query_id
        );
        for (field, value) in [
            ("passage title", passage.title.as_str()),
            ("passage heading", passage.heading.as_str()),
            ("passage text", passage.text.as_str()),
        ] {
            validate_model_visible_text(
                value,
                field,
                &[
                    query.query_id.as_str(),
                    judgment.passage_id.as_str(),
                    world_id,
                ],
            )?;
        }
        *passage_reference_counts
            .entry(judgment.passage_id.as_str())
            .or_default() += 1;
        reject_cross_split_normalized_value(
            normalized_passages,
            normalize_for_leakage(&model_passage_input(passage)),
            split,
            "passage model input",
        )?;
        match judgment.role {
            JudgmentRole::Answer => answer_count += 1,
            JudgmentRole::Support => support_count += 1,
            JudgmentRole::HardNegative => hard_negative_count += 1,
        }
    }

    let split_stats = stats
        .get_mut(&split)
        .context("selector split statistics are unavailable")?;
    split_stats.query_count += 1;
    *split_stats
        .language_directions
        .entry(LanguageDirection::new(query.language, passage_language))
        .or_default() += 1;
    for judgment in judgments {
        split_stats.tags.extend(judgment.tags.iter().copied());
        split_stats.disclosures.insert(judgment.disclosure);
        if judgment.role == JudgmentRole::Answer && judgment.disclosure == Disclosure::Forbidden {
            split_stats.forbidden_answer_count += 1;
        }
    }

    if answer_count == 0 {
        ensure!(
            support_count == 0 && hard_negative_count >= 4,
            "no-answer query `{}` must contain no answer/support and at least four hard negatives",
            query.query_id
        );
        split_stats.no_answer_count += 1;
    } else {
        ensure!(
            hard_negative_count >= 3,
            "answerable query `{}` must contain at least three hard negatives",
            query.query_id
        );
        if answer_count >= 2 {
            split_stats.multi_answer_count += 1;
        }
    }
    Ok(())
}

fn reject_cross_split_normalized_value(
    observed: &mut HashMap<String, Split>,
    value: String,
    split: Split,
    kind: &str,
) -> Result<()> {
    if let Some(previous_split) = observed.insert(value, split) {
        ensure!(
            previous_split == split,
            "normalized {kind} leaks across train and dev"
        );
    }
    Ok(())
}

fn validate_split_stats(
    stats: &BTreeMap<Split, SplitStats>,
    requirements: CorpusRequirements,
) -> Result<()> {
    let train = stats
        .get(&Split::Train)
        .context("train selector statistics are unavailable")?;
    let dev = stats
        .get(&Split::Dev)
        .context("dev selector statistics are unavailable")?;
    validate_one_split(
        Split::Train,
        train,
        requirements.train.query_count,
        requirements.train.no_answer_count,
        requirements.train.direction_count,
        requirements.train.minimum_multi_answer_count,
    )?;
    validate_one_split(
        Split::Dev,
        dev,
        requirements.dev.query_count,
        requirements.dev.no_answer_count,
        requirements.dev.direction_count,
        requirements.dev.minimum_multi_answer_count,
    )
}

fn validate_one_split(
    split: Split,
    stats: &SplitStats,
    expected_queries: usize,
    expected_no_answer: usize,
    expected_direction_count: usize,
    minimum_multi_answer: usize,
) -> Result<()> {
    ensure!(
        stats.query_count == expected_queries,
        "{} split must contain exactly {expected_queries} queries",
        split.label()
    );
    ensure!(
        stats.no_answer_count == expected_no_answer,
        "{} split must contain exactly {expected_no_answer} no-answer queries",
        split.label()
    );
    ensure!(
        stats.multi_answer_count >= minimum_multi_answer,
        "{} split must contain at least {minimum_multi_answer} answerable pools with two answers",
        split.label()
    );
    for direction in LanguageDirection::ALL {
        ensure!(
            stats
                .language_directions
                .get(&direction)
                .copied()
                .unwrap_or_default()
                == expected_direction_count,
            "{} split must contain exactly {expected_direction_count} {} queries",
            split.label(),
            direction.label()
        );
    }
    let missing_tags = REQUIRED_TAGS
        .iter()
        .filter(|tag| !stats.tags.contains(tag))
        .count();
    ensure!(
        missing_tags == 0,
        "{} split is missing {missing_tags} required taxonomy tags",
        split.label()
    );
    ensure!(
        stats.disclosures.contains(&Disclosure::Allowed)
            && stats.disclosures.contains(&Disclosure::Forbidden),
        "{} split must represent allowed and forbidden disclosure",
        split.label()
    );
    ensure!(
        stats.forbidden_answer_count > 0,
        "{} split must contain at least one forbidden answer",
        split.label()
    );
    Ok(())
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{kind} id is empty");
    ensure!(
        value.len() <= MAX_IDENTIFIER_BYTES,
        "{kind} id exceeds {MAX_IDENTIFIER_BYTES} bytes"
    );
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "{kind} id must use lowercase ASCII letters, digits, and underscores"
    );
    Ok(())
}

fn validate_nonempty_bounded_bytes(value: &str, maximum: usize, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{kind} is empty");
    ensure!(value.len() <= maximum, "{kind} exceeds {maximum} bytes");
    Ok(())
}

fn validate_nonempty_bounded_chars(value: &str, maximum: usize, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{kind} is empty");
    ensure!(
        value.chars().count() <= maximum,
        "{kind} exceeds {maximum} characters"
    );
    Ok(())
}

fn validate_model_visible_text(value: &str, kind: &str, internal_ids: &[&str]) -> Result<()> {
    let lowercase = value.to_ascii_lowercase();
    for identifier in internal_ids {
        ensure!(
            !lowercase.contains(&identifier.to_ascii_lowercase()),
            "{kind} contains internal corpus identifier `{identifier}`"
        );
    }
    ensure!(
        !lowercase
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(is_internal_generation_marker),
        "{kind} contains an internal split or candidate marker"
    );
    Ok(())
}

fn is_internal_generation_marker(token: &str) -> bool {
    token.starts_with("train_")
        || token.starts_with("dev_")
        || (token.len() == 3
            && token.starts_with('p')
            && token[1..].bytes().all(|byte| byte.is_ascii_digit()))
        || (token.len() == 4
            && matches!(token.as_bytes().first(), Some(b't' | b'd'))
            && token[1..].bytes().all(|byte| byte.is_ascii_digit()))
}

fn normalize_for_leakage(value: &str) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;
    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
        } else {
            if pending_space {
                normalized.push(' ');
                pending_space = false;
            }
            normalized.push(character);
        }
    }
    normalized
}

fn model_passage_input(passage: &Passage) -> String {
    [&passage.title, &passage.heading, &passage.text]
        .into_iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

type TemplateShingles = HashSet<Vec<String>>;

fn canonical_template_tokens(value: &str) -> Vec<String> {
    let raw_tokens = value
        .split(|character: char| {
            !(character.is_alphanumeric() || matches!(character, '_' | '-' | '.'))
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < raw_tokens.len() {
        if index + 1 < raw_tokens.len()
            && is_title_case_token(raw_tokens[index])
            && is_title_case_token(raw_tokens[index + 1])
        {
            let mut end = index + 2;
            while end < raw_tokens.len() && is_title_case_token(raw_tokens[end]) {
                end += 1;
            }
            push_collapsed_marker(&mut tokens, "<entity>");
            index = end;
        } else if raw_tokens[index]
            .chars()
            .any(|character| character.is_ascii_digit())
            || is_uppercase_synthetic_value(raw_tokens[index])
        {
            push_collapsed_marker(&mut tokens, "<slot>");
            index += 1;
        } else {
            tokens.push(raw_tokens[index].to_lowercase());
            index += 1;
        }
    }
    tokens
}

fn is_title_case_token(token: &str) -> bool {
    let mut characters = token.chars();
    characters.next().is_some_and(char::is_uppercase) && characters.any(char::is_lowercase)
}

fn is_uppercase_synthetic_value(token: &str) -> bool {
    let mut letters = token.chars().filter(|character| character.is_alphabetic());
    token.contains('-')
        && letters.clone().count() >= 4
        && letters.all(|character| character.is_uppercase())
}

fn push_collapsed_marker(tokens: &mut Vec<String>, marker: &str) {
    if tokens.last().is_none_or(|token| token != marker) {
        tokens.push(marker.to_owned());
    }
}

fn five_token_shingles(tokens: &[String]) -> TemplateShingles {
    tokens.windows(5).map(<[String]>::to_vec).collect()
}

fn templates_overlap(train: &TemplateShingles, dev: &TemplateShingles) -> bool {
    let shared = train.intersection(dev).count();
    let smaller = train.len().min(dev.len());
    let union = train.len() + dev.len() - shared;
    smaller >= 3 && shared >= 3 && shared * 5 >= smaller * 4 && shared * 20 >= union * 13
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{Value, json};

    use super::*;

    const TEST_DIRECTION_COUNT: usize = 1;
    const TEST_NO_ANSWER_COUNT: usize = 1;
    const TEST_QUERY_COUNT_PER_SPLIT: usize = TEST_DIRECTION_COUNT * 4;
    const TEST_REQUIREMENTS: CorpusRequirements = CorpusRequirements {
        train: SplitRequirements {
            query_count: TEST_QUERY_COUNT_PER_SPLIT,
            no_answer_count: TEST_NO_ANSWER_COUNT,
            direction_count: TEST_DIRECTION_COUNT,
            minimum_multi_answer_count: 1,
        },
        dev: SplitRequirements {
            query_count: TEST_QUERY_COUNT_PER_SPLIT,
            no_answer_count: TEST_NO_ANSWER_COUNT,
            direction_count: TEST_DIRECTION_COUNT,
            minimum_multi_answer_count: 1,
        },
    };

    struct TestCorpus {
        directory: tempfile::TempDir,
        queries: Vec<Value>,
        passages: Vec<Value>,
        judgments: Vec<Value>,
    }

    impl TestCorpus {
        fn valid() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let mut queries = Vec::new();
            let mut passages = Vec::new();
            let mut judgments = Vec::new();
            add_split(
                "train",
                TEST_DIRECTION_COUNT,
                TEST_NO_ANSWER_COUNT,
                &mut queries,
                &mut passages,
                &mut judgments,
            );
            add_split(
                "dev",
                TEST_DIRECTION_COUNT,
                TEST_NO_ANSWER_COUNT,
                &mut queries,
                &mut passages,
                &mut judgments,
            );
            let corpus = Self {
                directory,
                queries,
                passages,
                judgments,
            };
            corpus.write();
            corpus
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        fn write(&self) {
            write_jsonl(&self.root().join(QUERY_FILE), &self.queries);
            write_jsonl(&self.root().join(PASSAGE_FILE), &self.passages);
            write_jsonl(&self.root().join(JUDGMENT_FILE), &self.judgments);
        }
    }

    fn add_split(
        split: &str,
        direction_count: usize,
        no_answer_count: usize,
        queries: &mut Vec<Value>,
        passages: &mut Vec<Value>,
        judgments: &mut Vec<Value>,
    ) {
        let directions = [("es", "es"), ("en", "en"), ("en", "es"), ("es", "en")];
        let query_count = direction_count * directions.len();
        for query_index in 0..query_count {
            let (query_language, passage_language) = directions[query_index / direction_count];
            let query_id = format!("{split}_query_{query_index:03}");
            let world_id = format!("{split}_world_{query_index:03}");
            queries.push(json!({
                "query_id": query_id,
                "text": format!("{split} selector question {query_index}"),
                "language": query_language,
            }));
            let no_answer = query_index < no_answer_count;
            for judgment_index in 0..JUDGMENTS_PER_QUERY {
                let passage_id = format!("{split}_passage_{query_index:03}_{judgment_index}");
                let passage_text =
                    format!("Evidence for {split} query {query_index} candidate {judgment_index}.");
                passages.push(json!({
                    "passage_id": passage_id,
                    "title": format!("{split} title {query_index}"),
                    "heading": "Candidate set",
                    "text": passage_text,
                    "language": passage_language,
                }));
                let (role, answer_group_id, negative_kind, evidence_spans) = if no_answer {
                    (
                        "hard_negative",
                        Value::Null,
                        json!("wrong_entity"),
                        json!([]),
                    )
                } else {
                    match judgment_index {
                        0 | 1 => (
                            "answer",
                            json!(format!("group_{split}_{query_index:03}")),
                            Value::Null,
                            json!([{ "start": 0, "end": 8 }]),
                        ),
                        2 => ("support", Value::Null, json!("support_context"), json!([])),
                        _ => (
                            "hard_negative",
                            Value::Null,
                            json!("wrong_entity"),
                            json!([]),
                        ),
                    }
                };
                let tag = taxonomy_tag(
                    (query_index * JUDGMENTS_PER_QUERY + judgment_index) % REQUIRED_TAGS.len(),
                );
                let disclosure = if query_index == no_answer_count && judgment_index == 0 {
                    "forbidden"
                } else {
                    "allowed"
                };
                judgments.push(json!({
                    "query_id": query_id,
                    "passage_id": passage_id,
                    "split": split,
                    "world_id": world_id,
                    "role": role,
                    "answer_group_id": answer_group_id,
                    "disclosure": disclosure,
                    "tags": [tag],
                    "negative_kind": negative_kind,
                    "evidence_spans": evidence_spans,
                    "review_reason": "Synthetic selector judgment.",
                    "review_state": "synthetic_draft",
                }));
            }
        }
    }

    fn taxonomy_tag(index: usize) -> &'static str {
        [
            "direct",
            "paraphrase",
            "compound",
            "absence",
            "conflict",
            "duplicate",
            "injection_requested",
            "injection_unrelated",
            "wrong_entity",
            "wrong_relation",
            "wrong_date",
            "wrong_version",
            "wrong_scope",
            "negation",
            "ambiguity",
            "metadata_only",
            "long_passage",
        ][index]
    }

    fn write_jsonl(path: &Path, records: &[Value]) {
        let mut contents = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        contents.push('\n');
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn validator_accepts_the_complete_selector_contract() {
        let corpus = TestCorpus::valid();

        let summary =
            validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS).unwrap();

        assert_eq!(summary.judgments, TEST_QUERY_COUNT_PER_SPLIT * 2 * 6);
    }

    #[test]
    fn validator_rejects_unknown_fields() {
        let mut corpus = TestCorpus::valid();
        corpus.queries[0]["unexpected"] = json!(true);
        corpus.write();

        let error = format!(
            "{:#}",
            validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS).unwrap_err()
        );

        assert!(error.contains("unknown field"), "unexpected error: {error}");
    }

    #[test]
    fn sealed_file_check_rejects_modified_bytes() {
        let directory = tempfile::tempdir().unwrap();
        for (file_name, _) in SEALED_FILES {
            fs::write(directory.path().join(file_name), b"modified\n").unwrap();
        }

        let error = validate_sealed_files(directory.path())
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("create a new corpus version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_unknown_references() {
        let mut corpus = TestCorpus::valid();
        corpus.judgments[0]["passage_id"] = json!("missing_passage");
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("unknown passage"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_invalid_utf8_byte_spans() {
        let mut corpus = TestCorpus::valid();
        let passage_id = corpus.judgments[TEST_NO_ANSWER_COUNT * 6]["passage_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let passage = corpus
            .passages
            .iter_mut()
            .find(|passage| passage["passage_id"] == passage_id)
            .unwrap();
        passage["text"] = json!("Évidence with a multibyte first character.");
        corpus.judgments[TEST_NO_ANSWER_COUNT * 6]["evidence_spans"] =
            json!([{ "start": 1, "end": 8 }]);
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("UTF-8 byte boundaries"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_role_contract_mismatches() {
        let mut corpus = TestCorpus::valid();
        corpus.judgments[TEST_NO_ANSWER_COUNT * 6 + 2]["negative_kind"] = json!("wrong_entity");
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("support_context"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_role_specific_visible_metadata() {
        let mut corpus = TestCorpus::valid();
        corpus.passages[1]["title"] = json!("Positive answer");
        corpus.write();

        let error = validate_directory(corpus.root()).unwrap_err().to_string();

        assert!(
            error.contains("share neutral title and heading metadata"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_internal_ids_in_model_visible_text() {
        let mut corpus = TestCorpus::valid();
        let passage_id = corpus.passages[0]["passage_id"].as_str().unwrap();
        corpus.passages[0]["text"] = json!(format!("Internal marker {passage_id}"));
        corpus.write();

        let error = validate_directory(corpus.root()).unwrap_err().to_string();

        assert!(
            error.contains("contains internal corpus identifier"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_candidate_ordinals_in_model_visible_text() {
        let mut corpus = TestCorpus::valid();
        corpus.passages[0]["text"] = json!("Candidate p01 contains a leaked position.");
        corpus.write();

        let error = validate_directory(corpus.root()).unwrap_err().to_string();

        assert!(
            error.contains("internal split or candidate marker"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_split_coded_synthetic_values() {
        let mut corpus = TestCorpus::valid();
        corpus.passages[0]["text"] = json!("The synthetic record uses marker T031.");
        corpus.write();

        let error = validate_directory(corpus.root()).unwrap_err().to_string();

        assert!(
            error.contains("internal split or candidate marker"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn template_detector_rejects_same_pattern_with_different_synthetic_slots() {
        let train = five_token_shingles(&canonical_template_tokens(
            "An unapproved draft proposes 2033-01-03 for Proyecto Alondra Aliso.",
        ));
        let dev = five_token_shingles(&canonical_template_tokens(
            "An unapproved draft proposes 2044-12-11 for Iniciativa Prisma Vega.",
        ));

        assert!(templates_overlap(&train, &dev));
    }

    #[test]
    fn template_canonicalization_replaces_alphabetic_synthetic_codes() {
        assert_eq!(
            canonical_template_tokens(
                "The activation key for Proyecto Alondra Aliso is ALONDRA-ALISO."
            ),
            [
                "the",
                "activation",
                "key",
                "for",
                "<entity>",
                "is",
                "<slot>"
            ]
        );
    }

    #[test]
    fn template_detector_allows_short_shared_ordinary_phrasing() {
        let train = five_token_shingles(&canonical_template_tokens(
            "Human review keeps publication decisions explicit and local.",
        ));
        let dev = five_token_shingles(&canonical_template_tokens(
            "Human review keeps publication decisions explicit while source revisions remain auditable.",
        ));

        assert!(!templates_overlap(&train, &dev));
    }

    #[test]
    fn validator_rejects_normalized_cross_split_leakage() {
        let mut corpus = TestCorpus::valid();
        corpus.queries[TEST_QUERY_COUNT_PER_SPLIT]["text"] =
            json!("  TRAIN   SELECTOR QUESTION 0  ");
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("leaks across train and dev"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_cross_split_passage_input_leakage() {
        let mut corpus = TestCorpus::valid();
        let train_passage = corpus.passages[0].clone();
        let dev_passage = corpus
            .passages
            .iter_mut()
            .find(|passage| {
                passage["passage_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("dev_"))
            })
            .unwrap();
        dev_passage["title"] = train_passage["title"].clone();
        dev_passage["heading"] = train_passage["heading"].clone();
        dev_passage["text"] = train_passage["text"].clone();
        corpus.write();

        let error = validate_directory(corpus.root()).unwrap_err().to_string();

        assert!(
            error.contains("passage model input leaks across train and dev"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_worlds_outside_their_split() {
        let mut corpus = TestCorpus::valid();
        corpus.judgments[0]["world_id"] = json!("dev_world_wrong");
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("does not match split"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validator_rejects_missing_taxonomy_coverage() {
        let mut corpus = TestCorpus::valid();
        for judgment in corpus
            .judgments
            .iter_mut()
            .filter(|judgment| judgment["split"] == "dev")
        {
            if judgment["tags"] == json!(["long_passage"]) {
                judgment["tags"] = json!(["direct"]);
            }
        }
        corpus.write();

        let error = validate_directory_with_requirements(corpus.root(), TEST_REQUIREMENTS)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("required taxonomy tags"),
            "unexpected error: {error}"
        );
    }
}
