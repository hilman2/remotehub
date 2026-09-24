/**
 * Error texts for API problems (ADR 0002): the server sends a stable code,
 * the UI shows its message `error_<code>`. Indexing `m` with every generated
 * code makes a missing message a type error.
 */
import type { Locale } from '$lib/i18n';
import { m } from '$lib/paraglide/messages';
import { ERROR_CODES, type ErrorCode, type Problem } from './generated/problem';

export { ERROR_CODES, type ErrorCode, type Problem };

/** Code of the client for a server that did not answer at all. */
const NETWORK = 'network';

export function isErrorCode(code: unknown): code is ErrorCode {
	return typeof code === 'string' && (ERROR_CODES as readonly string[]).includes(code);
}

/** The message for a code; codes from a newer server fall back to a generic text. */
export function errorMessage(code: string, locale?: Locale): string {
	const options = locale ? { locale } : undefined;
	if (code === NETWORK) return m.server_unreachable({}, options);
	if (!isErrorCode(code)) return m.error_unknown({ code }, options);
	const message = m[`error_${code}`];
	return message({}, options);
}

/** Invalid fields whose problem has its own, more helpful text. */
const FIELD_MESSAGES = {
	private_key: m.error_field_private_key,
	passphrase: m.error_field_passphrase,
	certificate: m.error_field_certificate
} as const;

/** The message for a failed call: a field-specific text where there is one, else the code's. */
export function problemMessage(
	problem: { code: string; params?: Record<string, unknown> },
	locale?: Locale
): string {
	const field = problem.params?.field;
	if (
		problem.code === 'invalid_request' &&
		typeof field === 'string' &&
		Object.hasOwn(FIELD_MESSAGES, field)
	) {
		return FIELD_MESSAGES[field as keyof typeof FIELD_MESSAGES](
			{},
			locale ? { locale } : undefined
		);
	}
	return errorMessage(problem.code, locale);
}

/** Reads a problem response; null if the body is not one. */
export async function readProblem(response: Response): Promise<Problem | null> {
	try {
		const body: unknown = await response.json();
		if (body && typeof body === 'object' && 'code' in body && typeof body.code === 'string') {
			return body as Problem;
		}
	} catch {
		// not JSON
	}
	return null;
}
