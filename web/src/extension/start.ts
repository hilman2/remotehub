/**
 * Starts a page of the extension: in the language of the user's remotehub,
 * which connecting brought along, else the browser's; in remotehub's look.
 */
import { mount, type Component } from 'svelte';
import '../routes/layout.css';
import { getLocale, locales, setLocale, type Locale } from '$lib/i18n';
import { savedLocale } from './store';

export async function start(component: Component) {
	const saved = await savedLocale();
	if (saved && (locales as readonly string[]).includes(saved)) {
		setLocale(saved as Locale, { reload: false });
	}
	document.documentElement.lang = getLocale();
	mount(component, { target: document.body });
}
