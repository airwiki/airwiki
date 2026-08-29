# AirWiki desktop design guidelines

AirWiki uses an Apple-informed visual and interaction system adapted to a
cross-platform Tauri/WebView application. Apple Human Interface Guidelines are
a design reference, not a request to copy Apple branding, platform-exclusive
assets, or materials that the host does not actually provide.

Versioned source code, accessibility tests, and this document remain
authoritative. Local design skills may help contributors apply these rules, but
the contributor workflow must not depend on a skill installation.

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
- Treat network sharing and AI connections as distinct user concepts. **Share**
  contains only LAN and Internet exposure. The compact **AI Apps** status opens
  its own per-Wiki application-permission panel; Settings manages installation
  and connection health rather than duplicating those permissions.

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
- Keep adjacent buttons in a coherent group visually consistent.
- On macOS, use an ellipsis when a command opens another view that requires more
  input, when that convention remains clear after localization.

AirWiki uses its cross-platform icon system by default. SF Symbols can inform
icon semantics, weight, and alignment but must not be copied into unsupported
platforms or used outside Apple terms.

## Accessibility and validation

- Meet WCAG 2.2 AA: at least 4.5:1 for normal text and 3:1 for essential
  non-text controls, focus, and state indicators.
- Test keyboard-only operation, visible focus, reduced motion, appearance
  switching, localization expansion, and 200% text or display scaling where the
  shell supports it.
- Inspect realistic Library, Wiki content, and Settings screens at a constrained
  window near 1024×720, a comfortable window near 1180×760, and a wider window
  near 1440×900.
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
