# Synthetic acceptance corpus

These fictional files validate federation without exposing real organizational
or personal information.

- `mac/proyecto-atlas.md`: recovery procedure and technical validation.
- `mac/atlas-estado-verde.md` and `mac/atlas-estado-ambar.md`: contradictory
  status sources with no precedence rule.
- `mac/atlas-nota-no-confiable.md`: a harmless fact plus a hostile instruction
  that must remain untrusted data.
- `windows/proyecto-atlas-responsable.md`: synthetic owner and target date.
- `private/no-compartir.md`: an Orion canary that must never leave its local-only
  collection.
- `relevance/answerability.json`: schema-v2 relevance corpus with calibration,
  holdout, missing facts, contradictions, prompt injection, and cross-language
  cases.

The expected federated question asks how Atlas is recovered, who is responsible,
and the target date. A correct answer combines the Mac procedure with the
fictional Windows owner and date and cites each source node separately.

Always test invalidation on a copy. Change `validación sintética v3` to `v4` and
confirm that the published revision disappears until a human approves the new
one. Search for `ORION-PRIVATE-731` to test access control; no remote title,
metadata, or snippet may be returned.

Structural relevance validation downloads no model:

```bash
cargo run --locked -p xtask -- relevance validate
```

Real evaluation runs separately per platform against an already verified mMARCO
snapshot and writes aggregate synthetic evidence under `target/evals/`:

```bash
cargo run --locked -p xtask -- relevance evaluate --snapshot <snapshot-directory>
```

For MCP evaluation:

- the Mac and Windows fixture collections must be published, peer-shareable,
  granted where required, and explicitly enabled for external chat;
- `private/` must remain separate, local-only, and disabled for external chat;
- every new document requires manual review before the golden prompt set; and
- expected URNs, revisions, hashes, and PeerIds come from actual hits and are
  never hard-coded in this directory.
