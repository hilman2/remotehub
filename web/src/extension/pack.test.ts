import { createHash, createPublicKey, generateKeyPairSync, verify } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { unzipSync } from 'fflate';
import { describe, expect, it } from 'vitest';
import { extensionId, pack } from '../../scripts/pack-extension.mjs';

const DEV_KEY = join(import.meta.dirname, '../../extension-dev-key.pem');

/** The length-delimited fields of a protobuf message, by number. */
function fields(bytes: Buffer): Map<number, Buffer[]> {
	const found = new Map<number, Buffer[]>();
	let at = 0;
	const varint = () => {
		let value = 0;
		let shift = 0;
		for (;;) {
			const byte = bytes[at++];
			value += (byte & 0x7f) * 2 ** shift;
			if (byte < 0x80) return value;
			shift += 7;
		}
	};
	while (at < bytes.length) {
		const tag = varint();
		expect(tag & 7, 'only length-delimited fields').toBe(2);
		const length = varint();
		const number = Math.floor(tag / 8);
		found.set(number, [...(found.get(number) ?? []), bytes.subarray(at, at + length)]);
		at += length;
	}
	return found;
}

/** A build directory with a manifest carrying `publicKey`. */
function built(publicKey: string): string {
	const dir = mkdtempSync(join(tmpdir(), 'extension-'));
	writeFileSync(
		join(dir, 'manifest.json'),
		JSON.stringify({ manifest_version: 3, version: '1.2.3', key: publicKey })
	);
	mkdirSync(join(dir, 'icons'));
	writeFileSync(join(dir, 'icons', '16.png'), 'png');
	return dir;
}

const spki = (pem: Buffer) => createPublicKey(pem).export({ type: 'spki', format: 'der' });
const fromSpki = (der: Buffer) => createPublicKey({ key: der, format: 'der', type: 'spki' });

describe('packing the extension', () => {
	it('gives the development key the ID the development stack allows', () => {
		// deploy/compose.dev.yml and scripts/ci/lokal.sh name this ID.
		expect(extensionId(spki(readFileSync(DEV_KEY)))).toBe('obnekonmlefgdhgodgbjapgoophnhlao');
	});

	it('signs the archive it serves unpacked, as CRX3', () => {
		const publicKey = spki(readFileSync(DEV_KEY));
		const dir = built(publicKey.toString('base64'));
		const out = mkdtempSync(join(tmpdir(), 'packed-'));
		const info = pack(dir, out, DEV_KEY);
		expect(info).toEqual({ id: 'obnekonmlefgdhgodgbjapgoophnhlao', version: '1.2.3' });
		expect(JSON.parse(readFileSync(join(out, 'remotehub-extension.json'), 'utf8'))).toEqual(info);

		const archive = readFileSync(join(out, 'remotehub-extension.zip'));
		expect(Object.keys(unzipSync(archive)).sort()).toEqual(['icons/16.png', 'manifest.json']);

		const file = readFileSync(join(out, 'remotehub-extension.crx'));
		expect(file.subarray(0, 4).toString('latin1')).toBe('Cr24');
		expect(file.readUInt32LE(4)).toBe(3);
		const size = file.readUInt32LE(8);
		const header = fields(file.subarray(12, 12 + size));
		expect(Buffer.compare(file.subarray(12 + size), archive)).toBe(0);

		const signedData = header.get(10000)![0];
		const crxId = fields(signedData).get(1)![0];
		expect(crxId).toEqual(createHash('sha256').update(publicKey).digest().subarray(0, 16));

		const [proof] = header.get(2)!;
		const parts = fields(proof);
		expect(parts.get(1)![0]).toEqual(publicKey);
		const length = Buffer.alloc(4);
		length.writeUInt32LE(signedData.length);
		const signed = Buffer.concat([
			Buffer.from('CRX3 SignedData\x00', 'latin1'),
			length,
			signedData,
			archive
		]);
		expect(verify('sha256', signed, fromSpki(publicKey), parts.get(2)![0])).toBe(true);
		// A changed archive no longer matches the signature.
		signed[signed.length - 1] ^= 1;
		expect(verify('sha256', signed, fromSpki(publicKey), parts.get(2)![0])).toBe(false);
	});

	it('refuses a key other than the one in the manifest', () => {
		const other = generateKeyPairSync('rsa', { modulusLength: 2048 });
		const dir = built(other.publicKey.export({ type: 'spki', format: 'der' }).toString('base64'));
		expect(() => pack(dir, mkdtempSync(join(tmpdir(), 'packed-')), DEV_KEY)).toThrow(
			/not the key the build wrote/
		);
	});
});
