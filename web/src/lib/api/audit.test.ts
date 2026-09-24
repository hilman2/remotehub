import { describe, expect, it } from 'vitest';
import de from '../../../messages/de.json';
import en from '../../../messages/en.json';
import { locales } from '$lib/i18n';
import { AUDIT_ACTIONS, auditActionLabel } from './audit';

describe('audit action labels', () => {
	it('exist for every action in every locale', () => {
		for (const catalog of [en, de]) {
			for (const action of AUDIT_ACTIONS) {
				expect(Object.keys(catalog), action).toContain(`audit_${action.replaceAll('.', '_')}`);
			}
		}
	});

	it('render for every action in every locale', () => {
		for (const locale of locales) {
			for (const action of AUDIT_ACTIONS) {
				const label = auditActionLabel(action, locale);
				expect(label, `${locale}: ${action}`).not.toBe('');
				expect(label, `${locale}: ${action}`).not.toContain('audit_');
			}
		}
	});

	it('show unknown actions by name', () => {
		expect(auditActionLabel('vault.something_new', 'de')).toBe('vault.something_new');
	});
});
