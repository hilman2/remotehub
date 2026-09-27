// The offscreen document that clears the clipboard (clipboard.ts). It has no
// focus, so `navigator.clipboard` refuses; the `copy` command still works,
// and its event sets what it copies: nothing.
import { CLEAR_NOW } from './clipboard';

chrome.runtime.onMessage.addListener((message, sender, respond) => {
	if (sender.id !== chrome.runtime.id || message?.type !== CLEAR_NOW) return;
	document.addEventListener(
		'copy',
		(event) => {
			event.clipboardData?.setData('text/plain', '');
			event.preventDefault();
		},
		{ once: true }
	);
	document.execCommand('copy');
	respond(true);
});
