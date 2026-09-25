/**
 * The WebAuthn ceremonies for Kratos' `passkey` and `webauthn` methods
 * (#112), run by remotehub itself instead of Kratos' script. Kratos hands
 * the options as JSON in a node's value, with binary fields in base64url,
 * and takes the credential back in the same form.
 */

const decode = (text: string): Uint8Array<ArrayBuffer> =>
	Uint8Array.from(atob(text.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));

function encode(buffer: ArrayBuffer | null): string {
	if (!buffer) return '';
	return btoa(String.fromCharCode(...new Uint8Array(buffer)))
		.replaceAll('+', '-')
		.replaceAll('/', '_')
		.replaceAll('=', '');
}

/** Whether this browser can use passkeys and security keys at all. */
export const webauthnAvailable = () =>
	typeof window !== 'undefined' && 'PublicKeyCredential' in window;

type Descriptor = { id: string } & Record<string, unknown>;
const descriptors = (list?: Descriptor[]) => list?.map((d) => ({ ...d, id: decode(d.id) }));

/**
 * Creates a credential with the options of a node (`{ publicKey }`) and
 * returns it as Kratos takes it. Throws if the person cancels or the
 * authenticator refuses.
 */
export async function createCredential(options: string): Promise<string> {
	const { publicKey } = JSON.parse(options);
	const credential = (await navigator.credentials.create({
		publicKey: {
			...publicKey,
			challenge: decode(publicKey.challenge),
			user: { ...publicKey.user, id: decode(publicKey.user.id) },
			excludeCredentials: descriptors(publicKey.excludeCredentials)
		}
	})) as PublicKeyCredential;
	const response = credential.response as AuthenticatorAttestationResponse;
	return JSON.stringify({
		id: credential.id,
		rawId: encode(credential.rawId),
		type: credential.type,
		response: {
			attestationObject: encode(response.attestationObject),
			clientDataJSON: encode(response.clientDataJSON)
		}
	});
}

/** Signs a node's challenge (`{ publicKey }`) with a credential; see `createCredential`. */
export async function getCredential(options: string): Promise<string> {
	const { publicKey } = JSON.parse(options);
	const credential = (await navigator.credentials.get({
		publicKey: {
			...publicKey,
			challenge: decode(publicKey.challenge),
			allowCredentials: descriptors(publicKey.allowCredentials)
		}
	})) as PublicKeyCredential;
	const response = credential.response as AuthenticatorAssertionResponse;
	return JSON.stringify({
		id: credential.id,
		rawId: encode(credential.rawId),
		type: credential.type,
		response: {
			authenticatorData: encode(response.authenticatorData),
			clientDataJSON: encode(response.clientDataJSON),
			signature: encode(response.signature),
			userHandle: encode(response.userHandle)
		}
	});
}
