# W essential Windows validation

Date: 2026-09-07. Product revision:
`6e103356f724d952f7f863c53bc0923784948b90`.

The requested Windows visual and native-shell checks passed on this candidate.
The normal comparison requires native keyboard modality to be established before
the journey, as detailed below. This record does not approve a release or an
installer, or certify display scales beyond the current desktop configuration.

## Candidate

The Windows x64 E2E executable and bridge were copied to an isolated per-user
candidate directory. Tests used synthetic profiles. The daily installation and
its data remained unchanged. Application version: 0.3.0; Windows 11 Home 25H2,
build 26200.9278. Toolchain: Rust 1.96.1, Node 24.15.0, pnpm 10.18.3.
The recorded WebView identifier was `152.0.0.0`; the exact installed runtime
build was not independently recorded.

Installed executable SHA-256:
`c4c482a7b6c88ff5608bba8eebcdd9ddbb30ed99ee5fa2c1ffa76d75184cb168`.

## Completed checks

- PASS: frozen frontend dependency installation and native E2E build.
- PASS: normal capture journey, including session restoration after restart.
- PASS: review capture journey.
- PASS: final review journey with visual comparison enabled and
  `UPDATE_VISUAL_BASELINES=0` (one passing specification; 24 seconds).
- PASS: final normal comparison with visual comparison enabled and
  `UPDATE_VISUAL_BASELINES=0` after native keyboard preparation (one passing
  specification; 69 seconds), followed by session restoration after process
  restart (one passing specification; 2 seconds). The launcher exits with code 0.
- PASS: inspection of all 76 captured header regions, full representative views
  at the three sizes, and all three proposal views with differences outside the
  logo rectangle. No visible logo defect was found.
- PASS: native inspection of the application header and small title-bar icon.
- PASS: the candidate's actual taskbar icon shows the white W on its blue tile;
  its actual tray icon shows the bare blue W on the dark tray background.
- PASS: native Hide in tray removes the candidate window while its process
  remains alive. Activating the candidate's observed tray icon through native
  UI Automation InvokePattern restores the same window and article in that
  process. No executable was relaunched during this tray-restoration check.
- PASS: native Quit completely exits the candidate with code 0; the daily
  installation remains running.

The 76 captures comprise 48 normal reference views, 12 populated Library views
and 16 review views. English/Spanish and light/dark appearances cover actual
WebView sizes 1024x661, 1180x701 and 1440x841.

Exactly 52 Windows references were replaced with the reviewed native captures:
36 Library/reader/reader-sources views and 16 review views. The 12 Settings
references and all macOS references remain unchanged. Pixel comparison with the
previous references confines changes to the logo rectangle except four pixels
at approval-button corners across three proposal views; full-image inspection
found no visible change there. Product screenshots were not retouched.

## Focus-modality diagnosis

Two initial normal comparison attempts failed at `onboarding.spec.ts:1379`
(66 seconds each). The Clear-button assertion received
`{ focusVisible: false, contained: true }` instead of
`{ focusVisible: true, contained: true }`. This was a focus assertion failure,
not an image mismatch.

The installed `tauri-plugin-wdio-webdriver` 1.3.0 implementation in `executor.rs`
uses programmatic `click()` and `focus()` and dispatches synthetic keyboard/input
events. Those operations do not establish a real native keyboard interaction.
The assertion itself calls `focus()` before querying `:focus-visible`.

For the successful experiment, an external launcher paused after WebDriver
readiness and before starting each unmodified specification. The candidate was
activated with native input and received a real Tab key. Native observation
confirmed the visible focus ring on the onboarding language selector before the
normal journey, and on the sidebar toggle before the restoration journey. The
launcher then continued the existing tests with baseline updates disabled.

The same executable, references and assertions passed after this preparation.
Together with the driver implementation, this supports a harness precondition
gap around native input modality. No assertion, CSS, product code or versioned
test harness was changed, weakened or skipped. Unattended runs that omit native
keyboard preparation are not certified by this result.

## Native-shell method and limits

The window-oriented native tool did not enumerate Explorer's taskbar or tray.
The explicitly authorized Windows UI Automation fallback found the candidate's
taskbar button by its exact application identifier, distinct from the daily
installation. Bounded screen captures confirmed the taskbar W and the blue W in
the hidden-icon panel. The tray W was identified visually and by its observed
UI Automation runtime identifier, distinct from the daily installation's owl.
After hiding QA, its window was absent while the original process remained
alive. Invoking that exact tray button restored its original window, which was
then inspected through the window-oriented native tool. Small cropped icon
evidence remains local; no private application content was included.

An earlier input failure reported `call get_window_state before using this
window` despite refreshed observations. Resetting the JavaScript kernel and
selecting a fresh returned window recovered native clicks and keyboard input.
System permissions were not changed and no remote-desktop fallback was used.

Alternate operating-system display scales remain outside this acceptance. The
three-size WebView matrix does not substitute for native testing at other DPIs.

## Repository checks and scope

PASS: `git diff --check` and `cargo run --locked -p xtask --target
x86_64-pc-windows-msvc -- docs check`. The build reused a compatible cache with
incremental compilation and development debug information disabled.

The validation change contains the 52 Windows references and this report. The
approved references were not regenerated during the focus or tray follow-up.
The branch was rebased onto `faabdfd3c513f81b01ee2815f9884346f38b5969`, which adds
the separately reviewed MSI-reader fix and release notes. The tested product
revision and executable hash above remain unchanged. This validation adds no
product, versioned test-harness, macOS-reference or packaging modifications.
