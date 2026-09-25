import { describe, expect, it } from 'vitest';
import { generatePassword } from './generate';

describe('generated passwords', () => {
	it('are long, varied and without look-alikes', () => {
		const passwords = Array.from({ length: 200 }, () => generatePassword());
		expect(new Set(passwords).size).toBe(200);
		for (const password of passwords) {
			expect(password).toHaveLength(20);
			expect(password).toMatch(/[a-z]/);
			expect(password).toMatch(/[A-Z]/);
			expect(password).toMatch(/[2-9]/);
			expect(password).toMatch(/[-_.!?#%+=]/);
			expect(password).not.toMatch(/[0O1lI]/);
		}
		expect(generatePassword(32)).toHaveLength(32);
	});
});
