/**
 * Proof Key for Code Exchange (RFC 7636) for connecting the extension: the
 * code the connect page hands over is worth nothing without the verifier,
 * which never leaves the extension until it trades the code.
 */
import { base64url, randomBytes } from '$lib/vault/crypto';

/** A random value of `bytes` bytes in base64url, e.g. for `state`. */
export const randomToken = (bytes = 32) => base64url(randomBytes(bytes));

/** A verifier of 43 characters and its challenge with the method S256. */
export async function pkce(): Promise<{ verifier: string; challenge: string }> {
	const verifier = randomToken(32);
	const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
	return { verifier, challenge: base64url(new Uint8Array(digest)) };
}
