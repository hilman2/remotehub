/**
 * The generator's defaults (#194), loaded once per signed-in user and shared
 * by every password field of the page.
 */
import { loadGeneratorDefaults, type GeneratorDefaults } from '$lib/api/generator';
import { session } from '$lib/session.svelte';
import { BUILT_IN, type GeneratorSettings } from './generate';

export const defaults = $state<{ stored: GeneratorDefaults | null }>({ stored: null });

let loading: Promise<void> | null = null;
/** Whose defaults are stored: signing out does not reload the page. */
let loadedFor = '';

const whoIsSignedIn = () => (session.user ? `${session.user.kind}:${session.user.username}` : '');

/** Loads the defaults unless they are here; a failed load is tried again next time. */
export function loadDefaults(): Promise<void> {
	if (loadedFor !== whoIsSignedIn()) {
		loadedFor = whoIsSignedIn();
		defaults.stored = null;
		loading = null;
	}
	const who = loadedFor;
	loading ??= loadGeneratorDefaults().then((result) => {
		// An answer for whoever was signed in before is no one's now.
		if (who !== loadedFor) return;
		if (result.ok) defaults.stored = result.data;
		else loading = null;
	});
	return loading;
}

/** What the generator makes now: the user's own default, else the organisation's. */
export const current = (): GeneratorSettings =>
	defaults.stored?.own ?? defaults.stored?.organisation ?? BUILT_IN;
