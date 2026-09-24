// Runs in the device's page (Runtime.evaluate), called as
// fill(origin, which). Finds the sign-in form: the first visible password
// field and the last visible text field before it. Returns
// - { error: true } when Chromium shows its own error page instead (the
//   certificate is not the pinned one, the device does not answer),
// - null while the page is elsewhere or has no such field,
// - else { user: <whether a text field exists> }.
// With `which` ('user' or 'password') it also focuses that field and selects
// its content, so the text typed next replaces a prefilled value.
(origin, which) => {
	if (location.protocol === 'chrome-error:') return { error: true };
	if (location.origin !== origin) return null;
	const usable = (e) => e.getClientRects().length > 0 && !e.disabled && !e.readOnly;
	const inputs = [...document.querySelectorAll('input')].filter(usable);
	const password = inputs.find((e) => e.type === 'password');
	if (!password) return null;
	const user = inputs
		.slice(0, inputs.indexOf(password))
		.filter((e) => ['text', 'email', 'tel'].includes(e.type))
		.pop();
	const field = which === 'user' ? user : which === 'password' ? password : null;
	if (field) {
		field.focus();
		field.select();
		if (document.activeElement !== field) return null;
	}
	return { user: user !== undefined };
}
