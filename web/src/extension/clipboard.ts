/**
 * Clearing the clipboard after a secret was copied from the popup: the
 * popup asks the service worker (background.ts), which wakes after
 * `CLIPBOARD_SECONDS` and has the offscreen document clipboard.html
 * overwrite the clipboard.
 */

/** How long a copied secret stays in the clipboard, as in the vault's page. */
export const CLIPBOARD_SECONDS = 30;

/** Popup to worker: a secret was copied. */
export const CLEAR_CLIPBOARD = 'clear-clipboard';

/** Worker to the offscreen document: clear it now. */
export const CLEAR_NOW = 'clear-clipboard-now';

/** Asks for the clipboard to be cleared once `CLIPBOARD_SECONDS` have passed. */
export const clearLater = () => chrome.runtime.sendMessage({ type: CLEAR_CLIPBOARD });
