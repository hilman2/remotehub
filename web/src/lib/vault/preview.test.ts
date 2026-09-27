import { describe, expect, it } from 'vitest';
import { TEXT_LIMIT, preview } from './preview';

const bytes = (...parts: (string | number[])[]) =>
	new Uint8Array(
		parts.flatMap((part) => (typeof part === 'string' ? [...new TextEncoder().encode(part)] : part))
	);

describe('the file viewer', () => {
	it('shows text, whatever its name, and HTML only as text', () => {
		expect(preview('vpn.ovpn', bytes('remote vpn.example.com\n'))).toEqual({
			kind: 'text',
			text: 'remote vpn.example.com\n',
			truncated: false
		});
		expect(preview('page.html', bytes('<script>alert(1)</script>'))).toMatchObject({
			kind: 'text',
			text: '<script>alert(1)</script>'
		});
		expect(preview('empty', bytes(''))).toMatchObject({ kind: 'text', text: '' });
		expect(preview('umlaut.txt', bytes('Grüße'))).toMatchObject({ text: 'Grüße' });
	});

	it('reads UTF-16 with a byte order mark and old Windows text by its name', () => {
		expect(preview('export.csv', bytes([0xff, 0xfe, 0x41, 0, 0x42, 0]))).toMatchObject({
			kind: 'text',
			text: 'AB'
		});
		// "Grüße" in Windows-1252 is no UTF-8.
		const latin = bytes([0x47, 0x72, 0xfc, 0xdf, 0x65]);
		expect(preview('notes.txt', latin)).toMatchObject({ kind: 'text', text: 'Grüße' });
		expect(preview('notes.bin', latin)).toEqual({ kind: 'none' });
	});

	it('knows images and PDF by their first bytes, not by their names', () => {
		const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
		expect(preview('scan.pdf', bytes(png))).toEqual({ kind: 'image', type: 'image/png' });
		expect(preview('photo', bytes([0xff, 0xd8, 0xff, 0xe0]))).toEqual({
			kind: 'image',
			type: 'image/jpeg'
		});
		expect(preview('a.gif', bytes('GIF89a', [1, 0]))).toMatchObject({ type: 'image/gif' });
		expect(preview('a.webp', bytes('RIFF', [0, 0, 0, 0], 'WEBPVP8 '))).toMatchObject({
			type: 'image/webp'
		});
		expect(preview('invoice.png', bytes('%PDF-1.7\n'))).toEqual({ kind: 'pdf' });
	});

	it('takes weak signatures only with the matching name', () => {
		expect(preview('a.bmp', bytes('BM', [0, 0]))).toMatchObject({ type: 'image/bmp' });
		expect(preview('BMW.txt', bytes('BMW 320d'))).toMatchObject({ kind: 'text' });
		expect(preview('a.ico', bytes([0, 0, 1, 0, 1]))).toMatchObject({ type: 'image/x-icon' });
	});

	it('shows SVG only as an image, and only an SVG named so', () => {
		const svg = bytes('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>');
		expect(preview('logo.svg', svg)).toEqual({ kind: 'image', type: 'image/svg+xml' });
		expect(preview('logo.txt', svg)).toMatchObject({ kind: 'text' });
	});

	it('leaves other binary files to a download', () => {
		expect(preview('backup.zip', bytes('PK', [3, 4, 20, 0, 0, 0]))).toEqual({ kind: 'none' });
		expect(preview('key.der', bytes([0x30, 0x82, 0x01, 0x0a, 0x00]))).toEqual({ kind: 'none' });
	});

	it('shows no more than the limit of a long text', () => {
		const long = preview('huge.log', bytes('x'.repeat(TEXT_LIMIT + 5)));
		expect(long).toMatchObject({ kind: 'text', truncated: true });
		expect(long.kind === 'text' && long.text.length).toBe(TEXT_LIMIT);
	});
});
