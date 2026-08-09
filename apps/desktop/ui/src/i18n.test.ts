import { describe, expect, it } from 'vitest';
import { message, resolveLocale } from './i18n';

describe('Fluent UI localization', () => {
  it('resolves explicit and system locales deterministically', () => {
    expect(resolveLocale('es', 'en-US')).toBe('es');
    expect(resolveLocale('en', 'es-UY')).toBe('en-US');
    expect(resolveLocale('system', 'es-UY')).toBe('es');
    expect(resolveLocale('system', 'fr-FR')).toBe('en-US');
  });

  it('formats the same committed resources used by native surfaces', () => {
    expect(message('es', 'nav-review')).toBe('Revisión');
    expect(message('en', 'nav-review')).toBe('Review');
    expect(message('en', 'collections-counts', { documents: 3, published: 2 }))
      .toBe('3 documents · 2 published');
  });

  it('fails visibly for an unknown key', () => {
    expect(message('en', 'missing-synthetic-message')).toBe('missing-synthetic-message');
  });
});
