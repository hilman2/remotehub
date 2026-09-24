import { describe, expect, it } from 'vitest';
import { AUTH_MODES, KEYBOARD_LAYOUTS, authModesFor } from '$lib/api/catalog';
import { locales } from '$lib/i18n';
import { AUTH_MODE_LABELS, durationLabel, keyboardLayoutLabel } from './labels';

describe('sign-in modes', () => {
	it('fit the protocol', () => {
		expect(authModesFor('ssh')).toEqual(AUTH_MODES);
		expect(authModesFor('rdp')).toEqual(['stored', 'ask', 'own', 'laps']);
		expect(authModesFor('vnc')).toEqual(['stored', 'ask', 'own']);
		for (const mode of AUTH_MODES) expect(AUTH_MODE_LABELS[mode]()).not.toBe('');
	});
});

describe('duration labels', () => {
	it('count hours in the UI language', () => {
		expect(durationLabel(60, 'en')).toBe('1 hour');
		expect(durationLabel(1440, 'en')).toBe('24 hours');
		expect(durationLabel(240, 'de')).toBe('4 Stunden');
	});
});

describe('keyboard layout labels', () => {
	it('name language and region in the UI language', () => {
		expect(keyboardLayoutLabel('de-de-qwertz', 'de')).toBe('Deutsch (Deutschland) · QWERTZ');
		expect(keyboardLayoutLabel('de-de-qwertz', 'en')).toBe('German (Germany) · QWERTZ');
		expect(keyboardLayoutLabel('es-latam-qwerty', 'en')).toBe('Spanish (Latin America) · QWERTY');
		expect(keyboardLayoutLabel('failsafe', 'en')).toContain('Unicode');
	});

	it('exist for every layout and differ from each other', () => {
		for (const locale of locales) {
			const labels = KEYBOARD_LAYOUTS.map((layout) => keyboardLayoutLabel(layout, locale));
			expect(new Set(labels).size, locale).toBe(KEYBOARD_LAYOUTS.length);
		}
	});
});
