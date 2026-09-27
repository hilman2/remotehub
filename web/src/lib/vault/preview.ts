/**
 * What the vault's file viewer (#200) shows, and how. It decides by the
 * file's first bytes and its name, never by a type the file claims, and
 * renders nothing that could run in the page: HTML is shown as text, SVG
 * only as an image, where its scripts do not run.
 */

export type Preview =
	| { kind: 'text'; text: string; truncated: boolean }
	| { kind: 'image'; type: string }
	| { kind: 'pdf' }
	| { kind: 'none' };

/** The most text shown; the rest is left to a download. */
export const TEXT_LIMIT = 1_000_000;

/** Names whose content is text even when it is not UTF-8: old Windows files. */
const TEXT_NAMES =
	/\.(txt|log|csv|tsv|ini|inf|cfg|conf|config|reg|rdp|ovpn|bat|cmd|ps1|vbs|sh|env|properties)$/i;

const startsWith = (bytes: Uint8Array, prefix: number[], at = 0) =>
	prefix.every((byte, index) => bytes[at + index] === byte);
const ascii = (text: string) => [...text].map((c) => c.charCodeAt(0));

/** The image type of `bytes` by their signature; null for none. */
function imageType(name: string, bytes: Uint8Array): string | null {
	if (startsWith(bytes, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return 'image/png';
	if (startsWith(bytes, [0xff, 0xd8, 0xff])) return 'image/jpeg';
	if (startsWith(bytes, ascii('GIF87a')) || startsWith(bytes, ascii('GIF89a'))) return 'image/gif';
	if (startsWith(bytes, ascii('RIFF')) && startsWith(bytes, ascii('WEBP'), 8)) return 'image/webp';
	// Two letters are too weak a signature alone.
	if (/\.bmp$/i.test(name) && startsWith(bytes, ascii('BM'))) return 'image/bmp';
	if (/\.ico$/i.test(name) && startsWith(bytes, [0, 0, 1, 0])) return 'image/x-icon';
	return null;
}

/** `bytes` as text, if they are: UTF-8, UTF-16 with a byte order mark, or Windows-1252 by name. */
function textOf(name: string, bytes: Uint8Array): string | null {
	const decode = (encoding: string) => {
		try {
			return new TextDecoder(encoding, { fatal: true }).decode(bytes);
		} catch {
			return null;
		}
	};
	let text: string | null;
	if (startsWith(bytes, [0xff, 0xfe])) text = decode('utf-16le');
	else if (startsWith(bytes, [0xfe, 0xff])) text = decode('utf-16be');
	else text = decode('utf-8') ?? (TEXT_NAMES.test(name) ? decode('windows-1252') : null);
	// A NUL is binary: text has none.
	return text !== null && !text.includes('\u0000') ? text : null;
}

export function preview(name: string, bytes: Uint8Array): Preview {
	if (startsWith(bytes, ascii('%PDF-'))) return { kind: 'pdf' };
	const image = imageType(name, bytes);
	if (image) return { kind: 'image', type: image };
	const text = textOf(name, bytes);
	if (text === null) return { kind: 'none' };
	if (/\.svg$/i.test(name) && /<svg[\s>]/i.test(text)) {
		return { kind: 'image', type: 'image/svg+xml' };
	}
	return {
		kind: 'text',
		text: text.slice(0, TEXT_LIMIT),
		truncated: text.length > TEXT_LIMIT
	};
}
