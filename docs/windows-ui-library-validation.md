# Windows Library refinement validation

Date: 2026-09-07. Product revision: `cf2bcd76c919cf32aca0cdccb252ebbf4f89db85`.

The native Windows x64 E2E candidate passed the Library filter and two-line page
index acceptance. The executable and its bridge were copied to a separate
per-user candidate directory. Tests used isolated synthetic E2E profiles, without
models, real documents or configured public indexes. The daily installation was
left running and unchanged. This is UI acceptance, not installer, signing or
release acceptance.

Executable SHA256:
`91db2b6f5718fbe7a46af75c6637725b0182d1ecc8a2b5e091c0916afd019a44`.
Tooling: pinned Node 24.15.0 and pnpm 10.18.3; WebView2 152.0.0.0.

## Automated evidence

- PASS: frozen dependency installation and `build:e2e`.
- PASS: normal real-IPC journey and local session restoration after process restart.
- PASS: review journey.
- PASS: both journeys with visual comparison enabled and baseline updates disabled.
- Captured 76 native Windows matrix images: 48 normal reference views, 12 populated
  Library views and 16 review views. English/Spanish and light/dark appearances
  cover actual WebView sizes 1024x661, 1180x701 and 1440x841.
- Reviewed all 24 changed reader/source views and all 12 populated Library views.
  The 24 replacement references match the reviewed captures pixel for pixel.
  The 40 other Windows references and all macOS references remain unchanged.

The changed reference set is exactly the Cartesian product below, under
`apps/desktop/ui/e2e/baselines/win32/`:

| Filename component | Values |
| --- | --- |
| Locale | `en`, `es` |
| Theme | `light`, `dark` |
| View | `reader`, `reader-sources` |
| Size | `1024x661`, `1180x701`, `1440x841` |

Filename format: `{locale}-{theme}-{view}-{size}.png` (24 files).

## Native keyboard acceptance

These checks used Windows input in the candidate's actual WebView, separately
from WebDriver assertions:

- PASS: a filled name filter combines with the selected category; its result
  count changes while category counts retain the full Library totals.
- PASS: Tab puts a visible focus ring on Clear, contained within the input;
  Enter clears only the name, preserves the category and returns focus to the
  input. Immediate typing confirms that focus return.
- PASS: from the empty result state, Tab to Show all and Enter reset both filters
  and return focus to the input; immediate typing confirms it.
- PASS: checking status preserves both filters.
- PASS: native Back from reading and from Public restores the local filters.
- PASS: Back/Forward across two different local Library entries restores each
  entry's distinct name and category. Selecting a scope starts with clear filters.
- PASS: the candidate exits through Quit completely; the daily process remains.

The platform accessibility snapshot reports the WebView root as focused, so focus
acceptance relies on the visible caret/ring and successful immediate native typing,
not that accessibility field. No screen-reader acceptance is claimed. Native
keyboard checks used the light English candidate; the automated visual matrix
covers both supported languages and themes. No product or harness code changed.

## Repository checks

`git diff --check` and `cargo run --locked -p xtask --target
x86_64-pc-windows-msvc -- docs check` passed. An initial documentation build ran
out of disk space while writing an incremental cache (OS error 112). Only the
new incremental caches from that failed check were removed; after disk space
became available, the check passed with incremental compilation disabled.
The tested executable, references and existing artifacts were preserved.
