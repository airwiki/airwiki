# W essential Windows validation

Date: 2026-09-07. Product revision:
`6e103356f724d952f7f863c53bc0923784948b90`.

Validation is incomplete: capture journeys and the final review comparison passed,
but the normal comparison fails a focus assertion and native taskbar/tray-icon
acceptance remains pending. This record does not approve a release or an installer.

## Candidate

The Windows x64 E2E executable and bridge were copied to an isolated per-user
candidate directory. Tests used synthetic profiles. The daily installation and
its data remained unchanged. Application version: 0.3.0; Windows 11 Home 25H2,
build 26200.9278. Toolchain: Rust 1.96.1, Node 24.15.0, pnpm 10.18.3.

Installed executable SHA-256:
`c4c482a7b6c88ff5608bba8eebcdd9ddbb30ed99ee5fa2c1ffa76d75184cb168`.

## Completed checks

- PASS: frozen frontend dependency installation and native E2E build.
- PASS: normal capture journey, including session restoration after restart.
- PASS: review capture journey.
- PASS: final review journey with visual comparison enabled and
  `UPDATE_VISUAL_BASELINES=0` (one passing specification; 24 seconds).
- PASS: inspection of all 76 captured header regions, full representative views
  at the three sizes, and all three proposal views with differences outside the
  logo rectangle. No visible logo defect was found.
- PASS: native inspection of the application header and small title-bar icon.
- PASS: native Hide in tray removes the candidate window while its process
  remains alive. Opening the same candidate again restores the existing window
  and article; the second launcher exits successfully. This checks the
  single-instance restoration path, not a click on the tray icon.
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

## Failed and pending acceptance

- FAIL: two fresh normal journeys with visual comparison enabled and
  `UPDATE_VISUAL_BASELINES=0` stop at `onboarding.spec.ts:1379` (66 seconds each).
  The Clear-button assertion receives `{ focusVisible: false, contained: true }`
  instead of `{ focusVisible: true, contained: true }`. This is an assertion
  failure, not an image mismatch. The test focuses the button programmatically
  and checks `:focus-visible`; this run does not establish the cause of the
  differing focus-visible state. The normal comparison and its subsequent
  session-restoration run therefore cannot be marked PASS. No assertion was
  weakened or skipped, and no unrelated product fix was introduced.
- Native taskbar icon, tray icon and restoration from the tray are unverified.
  The native tool exposes application windows but no targetable taskbar or tray.
- Alternate operating-system display scales are unverified. The size matrix
  does not substitute for native DPI testing.
- An earlier input failure reported `call get_window_state before using this
  window` despite refreshed observations. Resetting the JavaScript kernel and
  selecting a fresh returned window recovered native clicks and keyboard input.
  System permissions were not changed and no remote-desktop fallback was used.

## Repository checks and scope

PASS: `git diff --check` and `cargo run --locked -p xtask --target
x86_64-pc-windows-msvc -- docs check`. The build reused a compatible cache with
incremental compilation and development debug information disabled.

The change contains the 52 Windows references and this report. No product,
test-harness, macOS-reference or packaging code was modified. The outstanding
normal-journey and native-shell gates must be resolved before full acceptance.
