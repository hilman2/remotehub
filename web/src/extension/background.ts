/**
 * The extension's service worker (#201, ADR 0017): the keyboard shortcut
 * and the context menu of text fields. Both fill the only login of the page
 * at once; with none or several, the popup opens to choose. It also clears
 * the clipboard after a secret was copied, which the popup cannot: it is
 * gone by then. The popup and the options page do everything else
 * themselves.
 */
import { locales, type Locale } from '$lib/i18n';
import { m } from '$lib/paraglide/messages';
import { CLEAR_CLIPBOARD, CLEAR_NOW, CLIPBOARD_SECONDS } from './clipboard';
import { allLogins, fillLogin, forPage, tabOrigin } from './logins';
import { connection, savedLocale } from './store';

const MENU = 'fill';

async function locale(): Promise<Locale | undefined> {
	const saved = await savedLocale();
	return (locales as readonly string[]).includes(saved ?? '') ? (saved as Locale) : undefined;
}

async function createMenu() {
	const language = await locale();
	await chrome.contextMenus.removeAll();
	chrome.contextMenus.create({
		id: MENU,
		title: m.extension_menu_fill({}, language ? { locale: language } : undefined),
		contexts: ['editable']
	});
}

chrome.runtime.onInstalled.addListener(() => {
	createMenu();
});
chrome.runtime.onStartup.addListener(() => {
	createMenu();
});
// The menu speaks the language of the user's remotehub, which connecting
// brings along.
chrome.storage.onChanged.addListener((changes, area) => {
	if (area === 'local' && changes.locale) createMenu();
});

/**
 * Fills the page's only login. Both callers came from the user in this tab,
 * which gives the extension access to it (`activeTab`).
 */
async function quickFill(tab: chrome.tabs.Tab | undefined) {
	if (!tab || !(await connection())) {
		await chrome.action.openPopup();
		return;
	}
	const { logins } = await allLogins();
	const found = forPage(logins, tabOrigin(tab));
	if (found.length !== 1 || !(await fillLogin(tab, found[0])).ok) {
		await chrome.action.openPopup();
	}
}

chrome.commands.onCommand.addListener((command, tab) => {
	if (command === 'fill') quickFill(tab);
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
	if (info.menuItemId === MENU) quickFill(tab);
});

// A copied password or code leaves the clipboard after half a minute, as in
// the vault's page. A timer would die with the worker, an alarm does not.
// Only a document can write the clipboard: an offscreen one, for a moment.
chrome.runtime.onMessage.addListener((message, sender) => {
	if (sender.id !== chrome.runtime.id || message?.type !== CLEAR_CLIPBOARD) return;
	chrome.alarms.create(CLEAR_CLIPBOARD, { delayInMinutes: CLIPBOARD_SECONDS / 60 });
});

chrome.alarms.onAlarm.addListener(async (alarm) => {
	if (alarm.name !== CLEAR_CLIPBOARD) return;
	if (!(await chrome.offscreen.hasDocument())) {
		await chrome.offscreen.createDocument({
			url: 'clipboard.html',
			reasons: [chrome.offscreen.Reason.CLIPBOARD],
			justification: 'Clears a copied password from the clipboard.'
		});
	}
	await chrome.runtime.sendMessage({ type: CLEAR_NOW });
	await chrome.offscreen.closeDocument();
});
