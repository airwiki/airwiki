# AirWiki branding

**W essential** is the AirWiki identity: the existing linked W and its three
nodes, in white on a blue (`#1461e8`) rounded square. The approved shape is
unchanged from [`wiki-linked-w.svg`](../../docs/assets/wiki-linked-w.svg) and
[`WikiIcon.svelte`](../../apps/desktop/ui/src/components/WikiIcon.svelte).

[`airwiki-mark.svg`](airwiki-mark.svg) is the canonical application artwork.
Its 128-unit canvas has a 118-unit tile, a 5-unit transparent outer margin and
27-unit corner radii. The W retains its original stroke, node sizes and relative
positions. All raster and platform assets are generated from this vector:

- `airwiki-mark.png` and `airwiki-app-icon.png`: 1024 px application artwork with transparent corners.
- `github-avatar.png`: square organization avatar.
- `github-social-preview.png`: repository social preview.
- `airwiki.icns`: multi-resolution macOS application and DMG icon.
- `airwiki.ico`: multi-resolution Windows application and installer icon.
- `airwiki-window.rgba`: 128 px RGBA application-window derivative.
- `airwiki-tray.rgba`: the bare linked W at 24 px, with blue strokes and transparent background. macOS uses its alpha as a template; Windows uses its color. The tile must not be included in this template.

The desktop UI, Tauri window icons, README, landing header/footer/favicon and
GitHub/social assets use these derivatives. Wiki controls keep the bare W in
`currentColor`; labels continue to convey their names and state.

## Regenerate and check

Use the repository's pinned Node.js and desktop UI dependencies, Python 3, and
the optional authoring dependency in [`requirements.txt`](requirements.txt).
No model, network call or operating-system icon utility is needed during
generation. From the repository root:

```bash
python3 -m pip install -r resources/branding/requirements.txt
python3 resources/branding/generate.py
python3 resources/branding/generate.py --check
```

`generate.py` uses the installed, pinned Tauri CLI to render SVG and build the
ICNS/ICO containers. Pillow encodes the remaining PNG/RGBA assets. Generation
first checks that the brand, reference vector and `WikiIcon` have identical W
geometry. `--check` regenerates in temporary storage and verifies every copy,
including desktop and landing assets, without modifying the repository.

Product screenshots and demo media must be recaptured from the current app;
do not paint a new logo over previous acceptance captures. Follow
[`CONTRIBUTING.md`](../../CONTRIBUTING.md) for native visual baselines and
[`site/README.md`](../../site/README.md) for the current media sources.

The branding files are distributed under the repository's Apache-2.0 license.
Preserve the approved vector when regenerating derivatives, and inspect small
sizes, light/dark appearances and native icon containers before replacing
packaged assets. Uploading the avatar/social image to an external profile and
publishing the landing or new installers are separate deployment actions.
