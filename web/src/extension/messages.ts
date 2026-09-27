/**
 * Texts for what can go wrong in the extension: the server's problem codes
 * as in remotehub's pages, and the extension's own, which never come from a
 * server.
 */
import { errorMessage } from '$lib/api/errors';
import { m } from '$lib/paraglide/messages';

const OWN: Record<string, () => string> = {
	extension_connect_cancelled: m.extension_connect_cancelled,
	extension_connect_failed: m.extension_connect_failed,
	extension_no_form: m.extension_no_form,
	extension_no_code_field: m.extension_no_code_field,
	extension_no_access: m.extension_no_access
};

export function message(code: string): string {
	return Object.hasOwn(OWN, code) ? OWN[code]() : errorMessage(code);
}
