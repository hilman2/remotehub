import { describe, expect, it } from 'vitest';
import { m } from '$lib/paraglide/messages';
import { kratosText, type UiText } from './flow';

const text = (id: number, type: UiText['type'] = 'error'): UiText => ({
	id,
	type,
	text: 'Kratos in English'
});

describe('Kratos messages', () => {
	it('come in the UI’s words by their ID (#232)', () => {
		expect(kratosText(text(4000015))).toBe(m.kratos_no_security_key());
		expect(kratosText(text(4000006))).toBe(m.error_invalid_credentials());
		expect(kratosText(text(4000034))).toBe(m.kratos_password_breached());
		expect(kratosText(text(1050001, 'success'))).toBe(m.kratos_saved());
		expect(kratosText(text(1010004, 'info'))).toBe(m.kratos_second_factor());
	});

	it('never show Kratos’ English', () => {
		// An error remotehub does not know, by its number.
		expect(kratosText(text(4999999))).toBe(m.kratos_error({ id: '4999999' }));
		expect(kratosText(text(4999999))).not.toContain('Kratos in English');
		// Any other message it does not know stays out.
		expect(kratosText(text(1070001, 'info'))).toBeNull();
		expect(kratosText(text(1999999, 'success'))).toBeNull();
	});
});
