import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

describe('desktop style tokens', () => {
  it('does not reference undeclared custom properties', () => {
    const stylesheet = readFileSync(
      resolve(process.cwd(), 'src/styles.css'),
      'utf8',
    );
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
});
