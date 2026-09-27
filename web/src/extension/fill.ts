/**
 * What the extension runs inside a page to find and fill a sign-in form
 * (#201, ADR 0017), through `chrome.scripting.executeScript`. Chromium turns
 * `pageForm` into source text and runs it in the page's frames, in the
 * extension's isolated world: the function may use nothing from outside its
 * own body, and gets its arguments as JSON.
 */

/** What a frame holds, or what was filled in it. */
export interface Form {
	user: boolean;
	password: boolean;
	code: boolean;
}

export interface Values {
	username?: string;
	password?: string;
	code?: string;
}

/**
 * Finds the sign-in form of this frame; with `values`, fills in what it
 * finds for them. Returns null when the frame is not at `origin`, the
 * origin of the page the login belongs to: frames of other sites, and
 * sandboxed ones, get nothing.
 *
 * The fields follow the rule of the browser service (crates/browser/src/
 * fill.js): the first password field and the last text field before it,
 * where the page's `autocomplete` hints do not name them. Only fields a
 * person can see and type into count.
 */
export function pageForm(origin: string, values: Values | null): Form | null {
	if (location.origin !== origin) return null;

	// A page can hide a field and still read what goes into it: transparent,
	// outside the page, a pixel wide, or inside something hidden. Such fields
	// are not filled.
	const usable = (input: HTMLInputElement) => {
		if (input.disabled || input.readOnly || input.type === 'hidden') return false;
		const box = input.getBoundingClientRect();
		if (box.width < 4 || box.height < 4 || box.right <= 0 || box.bottom <= 0) return false;
		return input.checkVisibility({
			opacityProperty: true,
			visibilityProperty: true,
			contentVisibilityAuto: true
		});
	};
	const inputs = [...document.querySelectorAll('input')].filter(usable);
	const hint = (input: HTMLInputElement, word: string) =>
		input.autocomplete.toLowerCase().split(/\s+/).includes(word);
	const textual = (input: HTMLInputElement) => ['text', 'email', 'tel', ''].includes(input.type);

	const passwords = inputs.filter((i) => i.type === 'password' && !hint(i, 'new-password'));
	const password = passwords.find((i) => hint(i, 'current-password')) ?? passwords[0];
	const named = /user|login|mail|account|benutzer|kennung/i;
	const user =
		inputs.find((i) => hint(i, 'username')) ??
		(password
			? inputs.slice(0, inputs.indexOf(password)).filter(textual).pop()
			: inputs.find((i) => i.type === 'email' || (textual(i) && named.test(`${i.name} ${i.id}`))));
	const focused = document.activeElement;
	const code =
		inputs.find((i) => hint(i, 'one-time-code')) ??
		inputs.find((i) => /otp|totp|2fa|mfa|one.?time/i.test(`${i.name} ${i.id}`)) ??
		(focused instanceof HTMLInputElement &&
		inputs.includes(focused) &&
		['text', 'tel', 'number', ''].includes(focused.type) &&
		focused !== user
			? focused
			: undefined);

	if (!values) return { user: !!user, password: !!password, code: !!code };

	// The element's own setter, then the events a person's typing sends:
	// frameworks that keep their own copy of a field's value (React, Vue)
	// take the new value from those events.
	const set = (input: HTMLInputElement, value: string) => {
		input.focus();
		const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
		if (setter) setter.call(input, value);
		else input.value = value;
		input.dispatchEvent(new Event('input', { bubbles: true }));
		input.dispatchEvent(new Event('change', { bubbles: true }));
	};
	const filled = { user: false, password: false, code: false };
	if (user && values.username !== undefined) {
		set(user, values.username);
		filled.user = true;
	}
	if (password && values.password !== undefined) {
		set(password, values.password);
		filled.password = true;
	}
	if (code && values.code !== undefined) {
		set(code, values.code);
		filled.code = true;
	}
	return filled;
}
