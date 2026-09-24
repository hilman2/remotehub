/**
 * Locale helpers around paraglide. UI texts come from messages/{locale}.json
 * (`import { m } from '$lib/paraglide/messages'`); numbers and dates are
 * formatted with Intl in the active locale.
 */
import { getLocale, locales, setLocale, type Locale } from '$lib/paraglide/runtime';

export { getLocale, locales, setLocale, type Locale };

/** Language names in their own language, so everyone finds theirs. */
export const LOCALE_NAMES: Record<Locale, string> = {
	en: 'English',
	de: 'Deutsch'
};

/** Regional fallback per UI language when the browser only says "en" or "de". */
const DEFAULT_REGION: Record<Locale, string> = {
	en: 'en-GB',
	de: 'de-DE'
};

/**
 * BCP 47 tag for Intl formatting: the browser's regional variant of the UI
 * language if it has one (en-US keeps its date order), otherwise a sensible
 * default with a 24-hour clock.
 */
export function formatLocale(
	locale: Locale = getLocale(),
	preferred: readonly string[] = typeof navigator === 'undefined' ? [] : navigator.languages
): string {
	const regional = preferred.find((tag) => tag.toLowerCase().startsWith(`${locale}-`));
	return regional ?? DEFAULT_REGION[locale];
}
