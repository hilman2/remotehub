// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces
declare global {
	namespace App {
		// interface Error {}
		// interface Locals {}
		// interface PageData {}
		/** Handed between the sign-in pages of local accounts (#103). */
		interface PageState {
			/** The Kratos settings flow to continue on /sign-in/setup. */
			settingsFlow?: string;
			/** Setup starts with a new password: after an invitation or recovery. */
			newPassword?: boolean;
			/** /sign-in starts with the second factor. */
			secondFactor?: boolean;
		}
		// interface Platform {}
	}
}

export {};
