# Desktop redesign: macOS validation

Validation date: 2026-09-06. Host: macOS 26.6.2 (25G83), arm64.
This records synthetic acceptance, without document content, queries, local
paths, identities, permissions, or raw application logs.

## Final candidate and checks

The final installed development executable has SHA-256:

```text
c0583f91eb3b8a2c5e045c8f7c77939da6a44426c275475b42ff74b5aa5e3490
```

Its product source is `c2df8310aa259fdfff7c537ab04166b681813f85` plus the
three-line folder-copy correction also committed on Windows as
`1f4a5f39e352a8c5cc6e8bf3328c7099f180bf43`. The installed executable was checked
byte-for-byte against the build output. The later capture helper waits for
fonts and the entry animation to settle; it does not change product code.

PASS: final native reading/Library/Sources/Settings visual comparison, review
visual comparison and second-process local restoration, without UPDATE.
All 16 final review captures were inspected in ES/EN, light/dark and three
window sizes before updating the eight compact references. The other 56 Mac
references remained byte-identical. The original 0.1% visual tolerance and
120 CSS pixel minimum initial editor area remain unchanged.

PASS: the four final onboarding captures at actual content sizes 1024 x 720
and 1180 x 740, in ES/EN, show the summary, next action and last note with the
footer visible. Initial captures caught the entry animation at partial or zero
opacity. Waiting for loaded fonts, full opacity and a finished animation
resolved the capture defect; the helper fails if the page does not settle.
The normal journey and restart passed again with that wait. Type checking and
independent review of the helper passed without weakening geometry assertions.

The integrated UI suite passed 322 tests before the final copy-only delta;
the seven existing onboarding tests, Svelte checking, lint, E2E types and build
passed after it. Earlier cross-cutting validation passed 1333 Rust tests with
one existing ignored test; the later excluded-proposal fix passed all 369 core
tests, Clippy and formatting. These counts identify those runs rather than
claiming every broad suite was repeated for the final wording change.

## Latest installed interactions

The immediately preceding installed executable, SHA-256
`238c05ea7ead86ad1b08add2f9307caa61909f17592e3cbe6d15a4bd2d343068`,
contains the same product source except the final folder-copy correction.
Native pointer, keyboard and wheel interaction confirmed:

- All onboarding steps in ES/EN at exterior 1024 x 720 and 1180 x 740. Long
  central content scrolls to its last note while Finish remains visible.
  Clicking Finish opens Library with no folder, model, license or new grant.
- Review comparison at exterior 1180 x 760 and compact Evidence/Proposal at
  1024 x 720. The summary is usable immediately and its bottom is reachable
  with the full-width action footer visible.
- An edited synthetic title survives switching panels and cancelling exit.
  Explicit discard returns to the queue without approval or exclusion.
- The original window size was restored and the candidate quit normally.

On the final executable, the folder step was inspected before and after
skipping in both ES and EN. The helper appears exactly once, never promises
local AI readiness, and Continue becomes available. Changing language and
returning preserves the skipped state. The copy quit normally through Cmd+Q.
No folder, model, license or permission was activated during this check.

## Earlier applicable installed acceptance

Earlier development candidates in this same redesign passed native interaction
for the unchanged portions of the final candidate:

- Reading with 50 concepts, long text, compact and wide Sources, Details,
  sidebar collapse/resize, and returning from Settings.
- Search without a prepared model: one-row input, actionable recovery in the
  results area, query retained on returning, and public search kept off.
- Approval, excluded-proposal recovery and stale-evidence failure: edits and
  queue position survive failure; restoring synthetic evidence permits retry
  and confirmed advancement without duplicate decisions.
- Native quit and cancelled quit preserve unsaved preferences or proposal
  edits. Reopening restores the validated local article and panel state.
- Details, Share and AI Apps return focus to their trigger after pointer or
  Escape dismissal. Cancelling nearby-access confirmation keeps it disabled.
- Sidebar widths of 224 and 200 pixels retain truncated names and counts;
  singular concept counts display correctly in ES/EN.
- Light/dark preferences, system reduced-motion behavior and a representative
  larger-text display preset retain usable reading/review controls. The display
  and system settings were restored afterward.

## Review and limits

A fresh independent context reviewed both complete PRs, publication/evidence,
permission and persistence boundaries, and subsequent focused corrections.
No actionable findings remain. The finding about capture CSS hiding invalid
field borders was fixed by excluding invalid fields from normalization.

Two earlier intermittent native automation failures at Sources opening and
panel-focus restoration passed unchanged retries. Their cause remains
unconfirmed; the later deterministic focus-generation race was separately
reproduced, fixed and verified. Final comparisons passed.

VoiceOver announcements and the exhaustive accessibility matrix remain
unverified, non-blocking follow-up under the product owner's revised priority.
The host offers display presets rather than literal 125/150/200% controls;
focused larger-text acceptance does not certify every percentage. This report
certifies neither Windows nor public signing, notarization or distribution.
See the separate [Windows report](desktop-redesign-windows-validation.md).
