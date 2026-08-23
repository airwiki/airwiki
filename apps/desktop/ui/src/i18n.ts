import { FluentBundle, FluentResource, type FluentVariable } from '@fluent/bundle';
import english from '../../locales/en-US.ftl?raw';
import spanish from '../../locales/es.ftl?raw';
import type { LocalePreference } from './generated/ui-contract';

export type MessageArgs = Record<string, FluentVariable>;
export type UiLocale = 'en-US' | 'es';

const sources: Record<UiLocale, string> = { 'en-US': english, es: spanish };
const bundles = new Map<UiLocale, FluentBundle>();

function bundleFor(locale: UiLocale): FluentBundle {
  const cached = bundles.get(locale);
  if (cached) return cached;
  const bundle = new FluentBundle(locale, { useIsolating: false });
  const errors = bundle.addResource(new FluentResource(sources[locale]));
  if (errors.length > 0) throw new Error(`Invalid ${locale} Fluent resource`);
  bundles.set(locale, bundle);
  return bundle;
}

export function resolveLocale(preference: LocalePreference, systemLocale = navigator.language): UiLocale {
  if (preference === 'es') return 'es';
  if (preference === 'en') return 'en-US';
  return systemLocale.toLowerCase().startsWith('es') ? 'es' : 'en-US';
}

export function message(
  preference: LocalePreference,
  id: string,
  args?: MessageArgs,
  systemLocale?: string
): string {
  const locale = resolveLocale(preference, systemLocale);
  const bundle = bundleFor(locale);
  const entry = bundle.getMessage(id);
  if (!entry?.value) return id;
  const errors: Error[] = [];
  const value = bundle.formatPattern(entry.value, args, errors);
  return errors.length === 0 ? value : id;
}
