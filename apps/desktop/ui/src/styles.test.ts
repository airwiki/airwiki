import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

function desktopStyles(): string {
  return readFileSync(
    resolve(process.cwd(), 'src/styles.css'),
    'utf8',
  );
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
});
