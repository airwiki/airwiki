# AirWiki desktop design guidelines

AirWiki uses an Apple-informed visual and interaction system adapted to a
cross-platform Tauri/WebView application. Apple Human Interface Guidelines are
a design reference, not a request to copy Apple branding, platform-exclusive
assets, or materials that the host does not actually provide.

Versioned source code, accessibility tests, and this document remain
authoritative. Local design skills may help contributors apply these rules, but
the contributor workflow must not depend on a skill installation.

## Cross-platform design contract

AirWiki presents one coherent product on macOS and Windows, not a pixel-identical
shell. The following are shared across platforms:

- information architecture, product terminology, and privacy and authorization
  boundaries;
- semantic tokens for appearance, typography roles, spacing, sizing, focus,
  selection, and status;
- core workflows, state and error/recovery behavior, and accessibility intent;
- keyboard-operable controls, semantic names and roles, and non-color-only
  communication of state.

The host platform adapts the presentation and interaction details: font stack
and text metrics; `Command` versus `Control` shortcut labels; focus and
selection treatment; and native window chrome, dialogs, file pickers, menus,
and notifications. A shared semantic token must therefore be mapped and tested
per platform rather than treated as a promise of identical pixels. Preserve the
native window's resize, system-menu, tile/snap, high-DPI, and accessibility
behavior; do not replace it with decorative WebView chrome.

## Design principles

- Keep content, knowledge state, and access boundaries visually primary.
- Use semantic roles for color, typography, spacing, sizing, focus, selection,
  and status. Map those roles independently in light and dark appearances.
- Prefer alignment, proximity, whitespace, and type hierarchy before adding a
  container, border, shadow, blur, or rounded shape.
- Preserve native window, keyboard, focus, dialog, file-picker, and shortcut
  behavior on each supported platform.
- Do not imitate Liquid Glass or another native material with fixed WebView blur
  and transparency. Use quiet opaque surfaces when native material is absent.
- Keep privacy, publication, AI access, and recovery state explicit in text and
  iconography; color is supporting information, never the only signal.
- Keep asynchronous progress visible at the point of action. A disabled control
  or busy pointer is not sufficient feedback: retain safe existing content and
  pair an indeterminate indicator with a concise verb when meaningful progress
  cannot be measured.
- Treat network sharing and AI connections as distinct user concepts. **Share**
  contains only LAN and Internet exposure. The compact **AI Apps** status opens
  its own per-Wiki application-permission panel; Settings manages installation,
  connection health and the app-specific **Search public knowledge** query-
  egress preference rather than duplicating local Wiki permissions.
- Explain LAN grants as access for the verified receiver device and its
  connected AI apps. Describe Internet controls only as publishing a Wiki.
  Never imply that connecting an AI app shares or publishes local knowledge.
- Keep public search off for each app until a native confirmation explains that
  its queries can leave the device. Show busy, error and retry state on that
  exact app row. A legacy-LAN migration notice remains non-blocking and
  disappears only after confirmed active grants are updated.

## Appearance roles

The compact palette below defines roles, not a license to scatter raw values
through components. Components consume the existing CSS custom properties.

| Role | Dark | Light |
| --- | --- | --- |
| Window background | `#101012` | `#F5F5F7` |
| Content surface | `#1C1C1E` | `#FFFFFF` |
| Raised or selected surface | `#2C2C2E` | `#E8E8ED` |
| Separator | `#38383A` | `#D1D1D6` |
| Primary label | `#F2F2F7` | `#1D1D1F` |
| Secondary label | `#AEAEB2` | `#6E6E73` |
| Interactive accent | `#0A84FF` | `#0066CC` |

Primary action fills may use a deeper blue than links or focus rings so their
label contrast remains at least 4.5:1. Success, warning, danger, progress, and
public-exposure colors have appearance-specific values and retain one meaning
throughout the product.

Use a base surface for the window and an elevated surface for panes, menus, and
selection. Do not tint every surface with the accent. Design and review light
and dark modes separately instead of deriving one by inversion.

## Typography

AirWiki deliberately keeps two product faces:

- **Atkinson Hyperlegible** for body and reading content.
- **Space Grotesk** for concise headings and product identity.

Use the platform system stack for compact control text when native familiarity
is more important than brand expression. Do not embed or redistribute Apple
system fonts; `-apple-system` and `system-ui` select the installed platform face.
On Windows, prefer `Segoe UI Variable` with `Segoe UI` as the system-font
fallback for compact UI text. Keep product display and reading faces in their
explicit roles, but never let them prevent system text scaling, localization,
or high-contrast readability.

Use these macOS-informed targets in logical CSS pixels, then validate them in
the installed application because CSS pixels and points can diverge with engine
and display scaling:

| Role | Target | Minimum line height |
| --- | ---: | ---: |
| Large page title | 26–32 | 32 |
| Section title | 17–22 | 22 |
| Body and control label | 13 | 16 |
| Callout or metadata | 11–12 | 14–15 |
| Persistent caption | 10 | 13 |

Avoid weights below Regular for interface text. Small labels use Medium or
Semibold. Reading content may use 15–17 pixels and a line height around 1.5–1.65.
No persistent explanatory text may be smaller than 10 pixels.

## Spacing and layout

Apple does not prescribe one universal eight-point grid for every macOS
interface. AirWiki uses a small product scale of `4, 8, 12, 16, 24, 32` logical
pixels and chooses among those values according to relationship:

- 4–8 pixels within one compact control or tightly related label/value pair.
- 8–12 pixels between controls in one command group.
- 16 pixels within a standard section or form group.
- 24 pixels between distinct conceptual groups or at pane gutters.
- 32 pixels for major page regions when the window has room.

Align panes, headings, rows, and controls to stable leading edges and shared
baselines. Put the most important information near the top and leading edge in
reading order. Keep reading widths comfortable, support continuous window
resizing, and avoid nested scroll regions. Never put the only critical action at
the bottom edge of a macOS window.

## Controls and targets

- Use 28 by 28 logical pixels as the normal macOS control target.
- Use 20 by 20 only as a compact exception with adequate spacing, a larger row
  hit region, and a keyboard path.
- Prefer 40–44 pixels for isolated primary actions or touch-capable hardware.
- Provide visible hover, pressed, disabled, selected, and keyboard-focus states.
- Reserve visible focus rings for interactive controls. A noninteractive route
  heading may receive programmatic focus for semantic navigation without
  drawing a full-width control outline.
- Keep adjacent buttons in a coherent group visually consistent.
- On macOS, use an ellipsis when a command opens another view that requires more
  input, when that convention remains clear after localization.

AirWiki uses its cross-platform icon system by default. SF Symbols can inform
icon semantics, weight, and alignment but must not be copied into unsupported
platforms or used outside Apple terms.

## Windows and Fluent adaptation

Windows uses the same product hierarchy and semantic roles, while retaining
Windows conventions. Use `Control` shortcut labels and conventional keyboard
interactions, including visible focus, logical tab order, arrow-key behavior in
composite controls, `Escape` for safe dismissal, and focus restoration after a
dialog closes. Do not rely on hover, a pointer, or a subtle selection tint as
the only way to find or operate a control.

Respect Windows system theme, high-contrast settings, text scaling, reduced
motion, and the user's configured accessibility behavior. In high contrast,
system-provided colors and focus indicators take priority over decorative
appearance; labels, boundaries, focus, and states must still be distinguishable
without color alone. Layout must reflow or expose secondary detail before text
or essential controls clip at enlarged text sizes.

Prefer native title-bar, system-menu, resizing, maximization, and snap-layout
behavior. Native dialogs and file pickers retain their host ownership and focus
rules. A custom title bar is acceptable only when it fully preserves these
behaviors, accessible hit targets, and scaling; it is not a reason to imitate
macOS traffic lights or toolbars. Fluent-inspired NavigationView and command
patterns may guide navigation when they fit the task, but they must not obscure
the product's Library, Wiki, Settings, or privacy concepts.

Fluent material is not a cross-platform visual default. Use quiet opaque
surfaces for dense or content-first panes on Windows as on macOS. Do not mimic
Mica, Acrylic, Liquid Glass, or another platform material with fixed blur,
transparency, or decorative highlights when the host does not provide it.

## Accessibility and validation

- Meet WCAG 2.2 AA: at least 4.5:1 for normal text and 3:1 for essential
  non-text controls, focus, and state indicators.
- Test keyboard-only operation, visible focus, reduced motion, appearance
  switching, localization expansion, and 125%, 150%, and 200% text or display
  scaling where the shell supports it. On Windows, also test high-contrast
  settings and platform focus treatment.
- Inspect realistic Library, Wiki content, and Settings screens at a constrained
  window near 1024×720, a comfortable window near 1180×760, and a wider window
  near 1440×900. Treat these as acceptance states for responsive behavior, not
  universal breakpoints or fixed platform metrics.
- Open the native window at 1180×760 by default, but keep the complete primary
  workflow usable down to its supported 1024×720 minimum for Split View and
  tiled desktop layouts.
- Treat screenshots as visual evidence, not as proof of accessible semantics or
  correct interaction.

## Apple references

- [Color](https://developer.apple.com/design/human-interface-guidelines/color)
- [Dark Mode](https://developer.apple.com/design/human-interface-guidelines/dark-mode)
- [Typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Layout](https://developer.apple.com/design/human-interface-guidelines/layout)
- [Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility)
- [Buttons](https://developer.apple.com/design/human-interface-guidelines/buttons)
- [SF Symbols](https://developer.apple.com/design/human-interface-guidelines/sf-symbols)
- [Apple Design Resources](https://developer.apple.com/design/resources/)

## Microsoft references

- [Typography](https://learn.microsoft.com/windows/apps/design/signature-experiences/typography)
- [High contrast](https://learn.microsoft.com/windows/apps/design/accessibility/high-contrast-themes)
- [Text scaling](https://learn.microsoft.com/windows/apps/develop/input/text-scaling)
- [Keyboard interactions](https://learn.microsoft.com/windows/apps/develop/input/keyboard-interactions)
- [NavigationView](https://learn.microsoft.com/windows/apps/develop/ui/controls/navigationview)
- [Title bar](https://learn.microsoft.com/windows/apps/develop/title-bar)
