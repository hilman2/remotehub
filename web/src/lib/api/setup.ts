/** The setup wizard of a fresh installation (#143). */
import { api } from './client';

/**
 * `pending` until the wizard has created the first administrator, then
 * `administrator` until they finish it, then `complete` for good.
 */
export type SetupPhase = 'pending' | 'administrator' | 'complete';

export interface SetupStatus {
	phase: SetupPhase;
	/** The last step done; the wizard continues after it. */
	step: number;
}

/** The wizard's steps, numbered as the server counts them. */
export const STEP = {
	administrator: 1,
	directory: 2,
	mail: 3,
	breakGlass: 4,
	recoveryKey: 5,
	done: 6
} as const;

/** The invitation of the new administrator: where and with what they set a password. */
export interface Invitation {
	link: string;
	code: string;
	expires_at: string;
}

/** Shown once, like the CLI shows it. */
export interface BreakGlassAccount {
	username: string;
	password: string;
	totp_secret: string;
	totp_uri: string;
}

export const loadSetupStatus = () => api<SetupStatus>('GET', '/api/setup');
export const checkCode = (code: string) => api('POST', '/api/setup/code', { code });
export const createAdministrator = (code: string, email: string, name: string) =>
	api<Invitation>('POST', '/api/setup/administrator', { code, email, name });
export const saveStep = (step: number) => api('PUT', '/api/setup/step', { step });
export const createBreakGlass = () => api<BreakGlassAccount>('POST', '/api/setup/break-glass');
export const completeSetup = () => api('POST', '/api/setup/complete');
