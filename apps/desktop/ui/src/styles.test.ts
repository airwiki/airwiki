import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

function desktopStyles(): string {
  return readFileSync(
    resolve(process.cwd(), 'src/styles.css'),
    'utf8',
  );
}

type Rgb = [number, number, number];

function customProperty(block: string, name: string): string {
  const value = block.match(new RegExp(`${name}\\s*:\\s*(#(?:[\\da-f]{6}|[\\da-f]{3}))`, 'i'))?.[1];
  if (!value) throw new Error(`${name} must be a hex color`);
  return value;
}

function rgb(color: string): Rgb {
  const expanded = color.length === 4
    ? `#${color.slice(1).split('').map((channel) => channel.repeat(2)).join('')}`
    : color;
  return [
    Number.parseInt(expanded.slice(1, 3), 16),
    Number.parseInt(expanded.slice(3, 5), 16),
    Number.parseInt(expanded.slice(5, 7), 16),
  ];
}

function blend(foreground: Rgb, background: Rgb, opacity: number): Rgb {
  return foreground.map((channel, index) => (
    channel * opacity + background[index] * (1 - opacity)
  )) as Rgb;
}

function luminance(color: Rgb): number {
  const [red, green, blue] = color.map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrast(first: Rgb, second: Rgb): number {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

describe('desktop style tokens', () => {
  it('does not reference undeclared custom properties', () => {
    const stylesheet = desktopStyles();
    const declarations = new Set(
      Array.from(stylesheet.matchAll(/(--[\w-]+)\s*:/g), (match) => match[1]),
    );
    const references = new Set(
      Array.from(stylesheet.matchAll(/var\(\s*(--[\w-]+)\s*\)/g), (match) => match[1]),
    );
    const undeclared = Array.from(references)
      .filter((token) => !declarations.has(token))
      .sort();

    expect(undeclared).toEqual([]);
  });

  it('keeps a shared wiki scrollable above the system status bar', () => {
    const stylesheet = desktopStyles();
    const sharedRule = stylesheet.match(/\.drive-page\.shared-wiki-open\s*{([^}]*)}/)?.[1];

    expect(sharedRule).toContain('overflow-y: auto');
    expect(sharedRule).not.toContain('overflow: clip');
  });

  it('keeps the sticky wiki toolbar flush with the scroll viewport', () => {
    const stylesheet = desktopStyles();
    const stickyRule = stylesheet.match(/\.wiki-content-sticky\s*{([^}]*)}/)?.[1];
    const stickyOffsets = Array.from(
      stylesheet.matchAll(/--wiki-sticky-offset:\s*(-\d+px)/g),
      (match) => match[1],
    );

    expect(stickyRule).toContain('top: var(--wiki-sticky-offset)');
    expect(stickyOffsets).toEqual(['-28px', '-24px']);
  });

  it('keeps long review content inside the review surface', () => {
    const stylesheet = desktopStyles();
    const drawerRule = stylesheet.match(/\.review-drawer\s*{([^}]*)}/)?.[1];
    const titleRule = stylesheet.match(/\.review-title-copy h2\s*{([^}]*)}/)?.[1];
    const evidenceRule = stylesheet.match(/\.evidence-list blockquote\s*{([^}]*)}/)?.[1];
    const comparisonRule = stylesheet.match(/\.review-comparison\s*{([^}]*)}/)?.[1];

    expect(drawerRule).toContain('overflow-x: hidden');
    expect(drawerRule).toContain('min-width: 0');
    expect(titleRule).toContain('overflow-wrap: anywhere');
    expect(evidenceRule).toContain('overflow-wrap: anywhere');
    expect(comparisonRule).toContain('minmax(260px, .88fr)');
  });

  it('keeps recovery guidance readable in every appearance', () => {
    const stylesheet = desktopStyles();
    const themeBlocks = Array.from(
      stylesheet.matchAll(/:root(?:\[data-theme='(?:light|system)'\])?\s*{([^}]*)}/g),
      (match) => match[1],
    ).filter((block) => block.includes('--recovery-accent'));
    expect(themeBlocks).toHaveLength(3);

    for (const block of themeBlocks) {
      const accent = rgb(customProperty(block, '--recovery-accent'));
      for (const surfaceName of ['--ink', '--slate']) {
        const surface = rgb(customProperty(block, surfaceName));
        const tintedSurface = blend(accent, surface, 0.08);
        expect(contrast(accent, tintedSurface), `${surfaceName} recovery contrast`).toBeGreaterThanOrEqual(4.5);
      }
    }

    const recoveryRule = stylesheet.match(/\.integration-recovery\s*{([^}]*)}/)?.[1];
    const recoveryLabelRule = stylesheet.match(/\.integration-recovery-label\s*{([^}]*)}/)?.[1];
    expect(recoveryRule).toContain('var(--recovery-accent)');
    expect(recoveryLabelRule).toContain('color: var(--recovery-accent)');
  });

  it('keeps light appearance text, actions, and control boundaries distinguishable', () => {
    const stylesheet = desktopStyles();
    const lightBlocks = Array.from(
      stylesheet.matchAll(/:root\[data-theme='light'\]\s*{([^}]*)}/g),
      (match) => match[1],
    );
    const lightTheme = lightBlocks.at(-1);
    expect(lightTheme).toBeDefined();
    if (!lightTheme) return;

    const windowSurface = rgb(customProperty(lightTheme, '--ink'));
    const contentSurface = rgb(customProperty(lightTheme, '--slate'));
    const primaryText = rgb(customProperty(lightTheme, '--strong'));
    const secondaryText = rgb(customProperty(lightTheme, '--muted'));
    const accent = rgb(customProperty(lightTheme, '--cyan'));
    const controlBorder = rgb(customProperty(lightTheme, '--control-border'));

    expect(contrast(primaryText, windowSurface), 'primary text on window').toBeGreaterThanOrEqual(4.5);
    expect(contrast(secondaryText, windowSurface), 'secondary text on window').toBeGreaterThanOrEqual(4.5);
    expect(contrast(secondaryText, contentSurface), 'secondary text on surface').toBeGreaterThanOrEqual(4.5);
    expect(contrast(accent, contentSurface), 'accent action on surface').toBeGreaterThanOrEqual(4.5);
    expect(contrast(controlBorder, contentSurface), 'control boundary on surface').toBeGreaterThanOrEqual(3);
  });

  it('keeps dark appearance text, actions, and control boundaries distinguishable', () => {
    const stylesheet = desktopStyles();
    const darkBlocks = Array.from(
      stylesheet.matchAll(/:root\s*{([^}]*)}/g),
      (match) => match[1],
    ).filter((block) => block.includes('--control-border'));
    const darkTheme = darkBlocks.at(-1);
    expect(darkTheme).toBeDefined();
    if (!darkTheme) return;

    const windowSurface = rgb(customProperty(darkTheme, '--ink'));
    const contentSurface = rgb(customProperty(darkTheme, '--slate'));
    const primaryText = rgb(customProperty(darkTheme, '--strong'));
    const secondaryText = rgb(customProperty(darkTheme, '--muted'));
    const accent = rgb(customProperty(darkTheme, '--cyan'));
    const controlBorder = rgb(customProperty(darkTheme, '--control-border'));

    expect(contrast(primaryText, windowSurface), 'primary text on window').toBeGreaterThanOrEqual(4.5);
    expect(contrast(secondaryText, windowSurface), 'secondary text on window').toBeGreaterThanOrEqual(4.5);
    expect(contrast(secondaryText, contentSurface), 'secondary text on surface').toBeGreaterThanOrEqual(4.5);
    expect(contrast(accent, contentSurface), 'accent action on surface').toBeGreaterThanOrEqual(4.5);
    expect(contrast(controlBorder, contentSurface), 'control boundary on surface').toBeGreaterThanOrEqual(3);
  });

  it('does not render persistent desktop labels below ten pixels', () => {
    const stylesheet = desktopStyles();
    const explicitSizes = Array.from(
      stylesheet.matchAll(/font-size:\s*([\d.]+)px/g),
      (match) => Number.parseFloat(match[1]),
    );
    const shorthandSizes = Array.from(
      stylesheet.matchAll(/font:\s*[^;{}]*?\s([\d.]+)px(?:\/|\s)/g),
      (match) => Number.parseFloat(match[1]),
    );

    expect([...explicitSizes, ...shorthandSizes].filter((size) => size < 10)).toEqual([]);
  });
});
