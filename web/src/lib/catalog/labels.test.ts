import { describe, expect, it } from 'vitest';
import { KEYBOARD_LAYOUTS } from '$lib/api/catalog';
import { locales } from '$lib/i18n';
import { keyboardLayoutLabel } from './labels';

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
