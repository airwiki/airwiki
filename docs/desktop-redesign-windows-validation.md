# Desktop redesign: Windows validation

## Follow-up: folder-step copy

Product commit: `1f4a5f39e352a8c5cc6e8bf3328c7099f180bf43`, based on the
previously recorded Windows candidate. This follow-up changes exactly three
product lines: the Spanish and English folder-skip helper and the condition
that hides its inline copy once the existing status paragraph is shown.

Installed executable SHA-256:

```text
ae660a989d7c2d4bc21d8cca06f86014ed09a4c33b3fc3e072b39c0793cbba76
```

**PASS: focused installed Windows interaction in Spanish and English.** A new
isolated development copy and empty temporary profile were used. The folder
step showed the helper once before skipping. After skipping, it appeared once
in the existing status paragraph and the inline helper was absent. Both
languages were inspected visually and through the native accessibility tree;
neither promised that local AI was ready. The Spanish post-skip state was
checked by returning to language selection after the English skip, choosing
Spanish, and revisiting the folder step with the skipped state preserved.
Continue became available after skipping.

No knowledge folder was selected or linked, no model or license was activated,
and no permission was granted. The copy was closed through its normal quit
dialog. Compilation (including Svelte checking) and diff checks passed.
Previously accepted journeys and visual comparisons were not repeated or
updated for this narrow follow-up. The earlier comparison and aria-invalid
limitation below still apply to their explicitly named candidate.

## Original acceptance record

Validation date: 2026-09-06. This report records completed observations; writing
the report did not rerun tests. It contains no machine paths, raw logs, document
content, or search queries.

## Candidate and scope

Final validated branch: `codex/windows-redesign-validation-db66ad`.
Final validated commit: `8b2c77cb1d3d18ad268dd30b0a546b520390268c`.

The installed executable used for final acceptance has SHA-256:

```text
fb0553b6f8dcd6d0565cb354753b2e8e25203838502d040fb346324effff2c74
```

This digest was calculated directly from the same installed executable after
validation, without rebuilding or rerunning tests. It also matches the retained
build output. The executable contains the product code committed in
`dd9a7e72f79cbcb36c916c07fa319ae769096262`; the final comparison used the test
code and visual references committed in
`8b2c77cb1d3d18ad268dd30b0a546b520390268c`. The later aria-invalid selector
exclusion remains outside this recorded comparison.

Relevant commits:

- `243b27b28e77d29034b367191c19426dfaf56340`: earlier Windows visual references
  and pointer positioning for review captures.
- `4dc23ee1b9832a881c67d5c2140dbbb7115291ed`: scrollable onboarding center with
  reachable navigation in short windows.
- `78775b5cb1d8e453ceca51ca534e0eb5f6c72f2c`: separate onboarding layout helper
  and regression coverage in the normal onboarding journey.
- `2082eff0f03f12079069afd5aa99f2986edac38d`: incoming compact review header.
- `6822067e24936a32bdb1524788620d1aca5140e5`: local integration of that header
  with the Windows onboarding work; this candidate still failed the review
  editor's initial-space assertion.
- `dd9a7e72f79cbcb36c916c07fa319ae769096262`: compact review spacing correction.
- `8b2c77cb1d3d18ad268dd30b0a546b520390268c`: 16 updated Windows review
  references and capture-only suppression of transient review-field hover borders.

Work used an isolated checkout, synthetic fixtures, isolated application data,
and a separately installed development copy. The normal installed application
and unrelated projects were preserved. No real document publication, peer
pairing, permission grants, model downloads, or external-AI activation were
part of this validation.

## Native E2E results

**PASS: final review comparison without `UPDATE_VISUAL_BASELINES`.** The runner
launched the separately installed development executable containing the product
code of `dd9a7e7`, using the final test code and references committed in `8b2c77c`.
It completed successfully with exit code 0.

All 16 review images were inspected across English and Spanish, light and dark
appearance, and the three matrix sizes. The matrix includes proposal and
evidence separately in the compact view, and comparison views at larger sizes.
The other 48 Windows references were verified byte-for-byte unchanged.

The original minimum of 120 CSS pixels of initially visible editor and the
visual mismatch tolerance of 0.1% were not reduced. All layout assertions
passed, including overflow, sidebar width, visible approval, footer alignment,
reserved content space, and reaching the editor's bottom after scrolling.

One intermediate comparison failed at 0.154%, limited to a transient editor
hover border. Capture-only CSS was added for review fields, the references were
regenerated, and the subsequent installed comparison passed without UPDATE.
This intermediate failure is not reported as a successful comparison.

The final review journey also passed retention of edits when abandoning a
close attempt, stale-evidence blocking, recovery after restoring synthetic
evidence, approval, exclusion, and reopening current evidence. These are E2E
results, not claims of a new manual run for every action on the final commit.

Compilation, E2E TypeScript checking, focused ESLint, and whitespace/diff checks
passed. Previously approved normal-journey and second-process restoration
comparisons were not rerun solely for the compact-header correction.

## Review geometry and installed interaction

Dimensions below are CSS content dimensions unless explicitly called exterior.

| Observation | Before compact spacing fix | After compact spacing fix |
| --- | --- | --- |
| E2E content at the smallest matrix size | 1024 x 661; initial-editor assertion failed | 1024 x 661; original 120 px assertion passed |
| Manually reached native minimum content | 1024 x 720 | 1024 x 720 |
| Initially visible editor at that native minimum | 158.515625 px | 214.3125 px |

Repeated native keyboard resizing stopped at 1024 x 720 content. The observed
exterior window was approximately 1026 x 772. The E2E helper compensated for
width but not height, so its requested size and actual content height differed.
The product CSS was corrected to pass the smaller E2E content area as well;
the test was not weakened to bypass this difference.

On the final installed product, the proposal was opened through native UI and
resized manually to its minimum. The editor began at 401.6875 px and the content
region ended at 616 px. Native wheel scrolling reached the full editor bottom
at 574.4375 px, inside that region; the action footer ended at 720 px and remained
visible. The validation copy was then closed through its normal quit dialog.

## Onboarding evidence

**PASS: installed onboarding regression and normal native journey.** The old
candidate reproduced the unreachable Finish action; the corrected candidate
passed the regression and completed the journey, including second-process
restoration. The original installed manual reproduction had a content height
of 740 px with Finish below the viewport. After correction, Finish was within
the viewport and the center scrolled independently; finishing required no
maximization.

All four saved onboarding images were inspected and then inspected again after
a report of blank-center captures on another platform:

- English at actual 1024 x 720 and 1180 x 740.
- Spanish at actual 1024 x 720 and 1180 x 740.

Every Windows image showed the central explanatory text, setup summary, local
AI next-step panel, and final informational note. None had a blank center.
The captures were taken after scrolling the center to its bottom. At 1024 x 720,
the large heading was partially clipped above the scroll region; at 1180 x 740
the heading was also visible. The footer remained visible in all four images.
This is a Windows observation and does not resolve the separately reported
macOS rendering concern.

## Actual Windows appearance and earlier manual acceptance

**PASS: actual operating-system appearance changed dark to light and back to
dark.** AirWiki remained configured to follow the system and visibly followed
both changes. The corresponding system color-scheme media query changed as
expected. The original Windows dark appearance was restored and Settings was
closed. This was an installed interaction, not merely an E2E preference change.

Earlier installed manual checks on the preceding redesign candidates passed:

- English and Spanish sidebar widths of 224 and 200 CSS pixels, with usable
  navigation, truncation, and visible counts; singular concept labels checked.
- Details, sharing, and AI-app panels opened and closed with focus returning
  to their origin, without granting access.
- Closing from Settings could be cancelled; quitting and reopening restored
  the prior reading destination.
- Review edits survived a cancelled close. A changed synthetic evidence file
  blocked approval while retaining edits; restoring it and retrying recovered.
- Actual Windows scaling at 125% and 150% retained usable review controls;
  the editor end remained reachable at 150%. Scaling was restored to 100%.
- Actual high-contrast appearance retained visible text, controls, and focus;
  the original non-high-contrast setting was restored.
- Actual reduced-motion setting changes were detected and animations were
  restored. Exact scroll restoration had automated coverage; a separate exact
  manual scroll-restoration measurement under reduced motion was not completed.

These earlier checks were not all repeated on `8b2c77c`. Final-commit installed
acceptance specifically covered the new review layout and its native E2E
comparison; the onboarding and system-appearance evidence remained applicable
to the unchanged portions of the candidate.

## Exact limits and pending work

- The later proposed `:not([aria-invalid="true"])` exclusion in hover-capture
  selectors was not executed in this Windows validation. The reported final
  comparison PASS applies to `8b2c77c`, before that subsequent change.
- Narrator control was blocked by the process integrity boundary. The user
  closed it. No Narrator PASS is claimed. Specialized assistive-technology
  validation and an exhaustive accessibility matrix were explicitly deferred
  as non-blocking work.
- A 200% scaling run was not performed; it was not offered by the observed
  standard selector. No PASS is claimed for that setting.
- This report does not certify macOS, subsequent commits, CI after integration,
  public packaging, signing, distribution, or a production release.
- No remaining action in the authorized Windows acceptance for `8b2c77c` was
  left unexecuted. The limitations above remain explicit rather than inferred
  successes.
