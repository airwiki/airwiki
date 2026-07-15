# macOS–Windows two-node acceptance runbook

This is the user-facing manual acceptance path. It uses only the application UI
and synthetic fixtures. Maintainer commands and evidence-recording requirements
are kept in the separate [maintainer validation guide](maintainer-validation.md).

Do not record PeerIds, IP addresses, ports, multiaddresses, local paths, queries,
snippets, SAS words, or document contents while running this procedure.

## Preparation

- Place the Mac and Windows PC on the same private subnet with multicast allowed.
- Use development candidates built from the same commit.
- Use copies of `fixtures/mac`, `fixtures/windows`, and `fixtures/private` only.
- Confirm that AirWiki reports the recommended local models as ready.
- On Windows, use only the two AirWiki firewall rules offered by the app;
  no rule may include the Public profile.

## Scenario

1. **Create isolated collections.** Create one Atlas collection on each node and
   one private Windows collection. Enable peer sharing and external chat only on
   the two Atlas collections. Keep both policies disabled on the private one.
2. **Automatic ingestion.** Wait for the startup scan or watcher. Do not force a
   manual rescan first. Each Atlas document must reach human review exactly once.
3. **Human publication.** Review the proposed metadata and publish the synthetic
   documents. Confirm that nothing was visible in Wiki, LAN, or MCP before
   approval.
4. **Local health.** Confirm that each published bundle is healthy and a local
   search returns relevant evidence without exposing source paths.
5. **Fresh discovery.** Revoke an inherited test relationship if present, then
   enable LAN on both nodes. The other device must appear automatically within
   ten seconds; do not use manual addresses.
6. **Pairing.** Start pairing from one node. Compare the same six SAS words on
   both screens and confirm on both devices before the deadline.
7. **Grants.** Grant only the remote Atlas collection in each direction. The
   private collection must not appear as an eligible grant.
8. **Federated search.** Ask how Atlas is recovered, who is responsible, and the
   target date. Evidence must combine the Mac procedure with Windows ownership
   and date, and every hit must contain the node, heading or page, revision, and
   source hash shown by the UI.
9. **Non-disclosure.** Search for the synthetic private canary
   `ORION-PRIVATE-731` from the Mac. No remote title, metadata, or snippet may be
   returned.
10. **New revision.** Modify only the copied Mac fixture. The published revision
    must disappear immediately from Wiki, LAN, and MCP, return to review, and
    become available again only after a new human approval.
11. **Offline behavior.** Exit AirWiki completely on Windows. A Mac search
    must finish within five seconds, explicitly report partial coverage, and not
    continue retrying after the result.
12. **Recovery.** Reopen Windows. Discovery and search must recover without a new
    SAS exchange; identity, trust, and grants must persist.
13. **Revocation.** Revoke Windows from the Mac. Active and later searches must
    stop receiving Windows evidence. Rediscovery alone must not restore trust or
    grants.
14. **Chat integration.** Pair again explicitly, grant only the Windows Atlas
    collection, and connect one supported desktop chat client from
    **Integrations**. The final answer must use `search_airwiki`, cite
    evidence from both nodes, declare partial coverage when applicable, and
    return no evidence for the private canary.

## Acceptance thresholds

- Zero unauthorized snippets, titles, or metadata.
- Local search over the synthetic corpus completes in under one second.
- Federated search completes in under three seconds when both nodes are online.
- An offline or partial MCP response completes in under five seconds.
- Discovery or reconnection completes in under ten seconds on the private LAN.
- No original, full document, index, embedding, path, or collection listing
  crosses the LAN.
- Publication, SAS confirmation, grants, external-chat policy, and re-publication
  always remain explicit human actions.

Stop at the first failed checkpoint. Record only the checkpoint's PASS/FAIL
result and elapsed time using the
maintainer guide. Keep any sanitized diagnostic detail local and outside the
acceptance record. Fix the smallest reproducible defect and restart from that
checkpoint plus the privacy and revocation checks.
