import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { readKdbx, writeKdbx, type KdbxEntry } from './kdbx';

/**
 * Made by KeePassXC 2 (keepassxc-cli, KDBX 4 with Argon2d): the group
 * Servers with the entry Router and its file vpn.txt.
 */
const FIXTURE = readFileSync(new URL('./fixtures/keepassxc.kdbx', import.meta.url));
const fixture = () =>
	FIXTURE.buffer.slice(FIXTURE.byteOffset, FIXTURE.byteOffset + FIXTURE.byteLength);

describe('KeePass files', () => {
	it('reads a file KeePassXC wrote', async () => {
		const [router, ...rest] = await readKdbx(fixture(), 'Fixture-Passw0rd');
		expect(rest).toEqual([]);
		expect(router).toMatchObject({
			path: ['Servers'],
			title: 'Router',
			username: 'admin',
			password: 'Entry-Pass!',
			url: 'https://router.lan'
		});
		expect(router.files.map((f) => [f.name, new TextDecoder().decode(f.data)])).toEqual([
			['vpn.txt', 'remote vpn']
		]);
	});

	it('refuses a wrong password', async () => {
		await expect(readKdbx(fixture(), 'wrong')).rejects.toThrow();
	});

	it('reads back what it wrote', async () => {
		const entries: KdbxEntry[] = [
			{
				path: [],
				title: 'Top',
				username: 'u',
				password: 'p',
				url: '',
				notes: 'line 1\nline 2',
				icon: 0,
				fields: [],
				files: []
			},
			{
				path: ['Bank', 'Cards'],
				title: 'Visa',
				username: 'me',
				password: 'Pässwörd 😀',
				url: 'https://bank.example',
				notes: '',
				icon: 66,
				fields: [
					{ name: 'PIN', value: '1234', protected: true },
					{ name: 'Branch', value: 'Main street', protected: false }
				],
				files: [{ name: 'card.txt', data: new TextEncoder().encode('front and back') }]
			}
		];
		const file = await writeKdbx(entries, 'Export-Passw0rd', 'remotehub');
		const back = await readKdbx(file, 'Export-Passw0rd');
		expect(back).toEqual(entries);
	});
});
