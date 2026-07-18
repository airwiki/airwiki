# Changelog

All notable user-visible changes to AirWiki will be documented here. The project has no supported public release yet, so current work remains under `Unreleased`.

## [Unreleased]

### Added

- Initial development baseline for local ingestion, human-reviewed OKF publication, LAN federation, and read-only local chat integration.
- Contributor guidance, architecture decisions, synthetic fixtures, and proportional validation commands.
- A proportional code-review contract, pull-request template, and read-only DCO validation for contributions.
- Shared AirWiki branding for GitHub, application windows, tray icons, and desktop packages.
- A guided first-knowledge journey from local AI setup through folder processing, human review, publication, and a cited local search.
- A revision-bound evidence panel that lets reviewers compare locally extracted text with each proposed draft before publication.
- Direct navigation from a local search result to its exact published Wiki concept, with the cited source location and revision kept visible.
- A reproducible retrieval-quality corpus and evaluator covering local and peer-authorized source evidence, abstention, provenance, privacy, contradictions, deduplication, and stable ordering.
- A content-free, pinned answerability corpus workflow and a development-only two-stage QA-entailment rejection gate with sanitized aggregate reports; production search remains unchanged.
- A development-only reviewed-evidence-anchor ablation comparing passage QA, reviewed-claim selection, and deterministic conflict detection without changing production search or OKF.
- A dependency-free, development-only Rust mini-graph ablation comparing wider hybrid retrieval with bounded expansion over reviewed internal OKF links.
- A real-ranking mini-graph replay that materializes healthy OKF bundles, reuses production BM25/E5/RRF, and compares bounded expansion with both a wider candidate pool and a degree-preserving sham graph.
- An independently authored, sealed multichunk holdout for evaluating compact OKF graph nomination through the existing relevance and final top-five citation path.
- A development-only mMARCO score-order diagnostic that compares the current relevance-filter order with descending cross-encoder order from one shared inference.
- A grouped, bilingual mMARCO cutoff-calibration corpus and evaluation-only score seam that keep third-party text and raw logits out of reports while comparing the current evidence policy with one frozen absolute threshold.
- A development-only two-hop OKF navigation diagnostic with a compact Rust graph, procedurally separated bilingual artifacts, and an exact weak-degree-preserving structural control.

### Changed

- Clarified privacy boundaries and separated internal development packages from future public distribution.
- Consolidated durable project guidance and documentation ownership.
- Improved the public project summary and contributor entry points.
- Kept LAN, chat clients, background operation, and updates out of the first useful-result path; they remain optional and require their existing confirmations.
- Reused a successful model integrity check for presentation state instead of hashing the same immutable assets twice during startup.
- Showed the local-model recommendation while private services finish starting on fresh installs, without weakening asset verification.
- Made the desktop layout responsive down to its explicit minimum size, keeping primary actions visible and limiting scrolling to bounded lists and detail panels.
- Made the first-run journey recoverable with visible processing counts, actionable terminal states, a truthful Wiki-health timestamp, and an explicit way to finish after a search returns no evidence.
- Separated retrieval development cases from disqualified holdout domains and added a fail-closed local selector experiment; production search remains unchanged because the measured candidate missed false-evidence and latency gates.
- Kept answerability references out of model control flow and scored accepted answers post-inference with the documented SQuAD/XQuAD normalized exact-match policy.
- Made retrieval research hypothesis-driven: literature supplies baselines and controls, while AirWiki-specific mechanisms require versioned ablations, counterfactual tests, fresh holdouts, and explicit rejection conditions.
- Kept production retrieval unchanged after the combined reviewed-claim representation and selector showed higher development coverage but failed false-evidence, no-answer, and correctness gates; observed latency remained descriptive.
- Recorded that the synthetic mini-graph mechanistic gate passed while keeping it at candidate stage until it is reproduced from healthy OKF bundles and real BM25/E5 rankings on fresh domains.
- Kept production retrieval unchanged after the real-ranking mini-graph development replay improved candidate-group coverage; a separately authored holdout and end-to-end chunk/reranker evidence remain required.
- Rejected compact OKF graph nomination for shadow use after the sealed end-to-end holdout matched the wider hybrid baseline and structural sham at 0.75 Recall@5 while exceeding its frozen candidate-assembly budget.
- Kept production retrieval unchanged after mMARCO score ordering produced the same visible-development results as the current filter order, identifying the relevance mask rather than ordering as the immediate bottleneck.
- Rejected a single absolute mMARCO logit cutoff after grouped bilingual calibration lost four baseline-complete queries and still returned a hard negative; no sealed holdout or production threshold was authorized.
- Rejected raw two-hop reviewed-link connectivity as a standalone retrieval decision after it connected every positive semantic pair but also 17 of 24 hard negatives; production search remains unchanged.

### Fixed

- Amended the sealed graph evaluator before observing outcomes so its deterministic degree-preserving sham retains mathematically unavoidable links instead of aborting on valid directed degree sequences.
- Made the first pending review immediately selectable and localized model diagnostics used by the normal setup flow.
- Showed files that could not be prepared alongside review drafts with a safe, localized cause and a folder rescan action.
- Removed flaky filesystem-watcher and reranker timing assumptions from macOS CI and made Windows hashing tests load their required PowerShell module explicitly.
- Replaced the GitHub advisory action that required unavailable check-write permission with a read-only `cargo audit` gate.
- Kept Wiki Health recovery actions honest: filesystem and ambiguous findings now require manual recovery instead of offering a guided repair that cannot complete, while missing pages are no longer rendered as dead links.

### Security

- External-chat access, peer sharing, grants, publication, and ambiguous repairs remain explicit human decisions and fail closed when authorization is uncertain.
- Remote search results remain read-only evidence cards and never expose a local Wiki navigation action.
