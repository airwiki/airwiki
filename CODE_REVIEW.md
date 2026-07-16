# Code review

AirWiki uses proportional review: the smallest process that provides credible evidence for the risk of the change. Review confirms privacy and correctness; it does not reward large diffs, speculative abstractions, or process for its own sake.

## Required flow

1. Create a focused branch from a known-good `main` revision.
2. Open a pull request that states the user-visible outcome, deliberate deferrals, and applicable evidence.
3. Complete the review tier below and resolve actionable findings.
4. Require every applicable CI and DCO check to be green.
5. Merge to `main`; never use a direct push to bypass failed or missing evidence.

Repository branch protection is an administrative control for this flow, not a substitute for it. Until protection is configured, maintainers still follow the same sequence manually.

## Review tiers

### Documentation only

Use this tier for prose, diagrams, and metadata that do not change runtime behavior, security policy, packaging, or a normative contract.

- Review the rendered meaning and local links.
- Run the documentation-only validation from `AGENTS.md`.
- A fresh author pass is sufficient; normative security or contribution policy uses the trust-boundary tier.

### Ordinary code

Use this tier for normal behavior within an existing architecture and trust model.

- Run focused or workspace validation proportional to the affected crates.
- Use a fresh reviewer context that did not implement the change. A maintainer remains responsible for accepting or rejecting the findings.
- Cover the success path and applicable failure and recovery paths.
- Perform the shortest installed-platform check when UI or operating-system behavior changes.

### Trust boundaries

Use this tier when a change can affect publication, authorization, LAN identity or grants, MCP or external-chat disclosure, local secrets, filesystem containment, migrations, the elevated firewall helper, updater trust, signing, logs, or a new dependency/data flow.

- Perform a dedicated security review against the threat model and relevant ADRs.
- Add negative tests for unauthorized, malformed, stale, revoked, or interrupted states as applicable.
- Run the relevant installed-platform and recovery checks.
- Update the threat model or add an ADR when the durable trust or authority model changes.
- Keep the change fail-closed until the evidence is complete.

## Reviewer checklist

- The diff has one clear outcome and leaves unrelated cleanup or future features out.
- Domain rules remain in the owning crate rather than widgets or transports.
- Publication, sharing, external AI, repairs, and source mutation still require their documented human authority.
- Error paths are typed and actionable at reusable boundaries; logs and evidence remain sanitized.
- Tests prove changed behavior rather than implementation details, including relevant recovery behavior.
- Documentation, `CHANGELOG.md`, threat model, migration notes, and ADRs are updated only where their ownership requires it.
- Dependencies, `Cargo.lock`, generated legal inventories, and platform packaging are reviewed together when any of them changes.

## Evidence hygiene

Follow [docs/maintainer-validation.md](docs/maintainer-validation.md). Pull requests may record the tested commit, package hash, versions, elapsed time, and PASS/FAIL. They must not include document content, queries, snippets, PeerIds, addresses, ports, multiaddresses, SAS words, local paths, usernames, databases, credentials, or raw application logs.

When evidence cannot be shared safely, record the sanitized result and keep the diagnostic material local. A missing artifact never justifies weakening a privacy boundary.
