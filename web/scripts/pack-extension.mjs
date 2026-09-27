/**
 * Packs the built browser extension (#201) for remotehub to serve
 * (crates/server/src/downloads.rs):
 *
 * - remotehub-extension.zip: the build as it is, to load unpacked by hand
 * - remotehub-extension.crx: the same archive signed (CRX3), for browsers
 *   that install it by policy from remotehub's update manifest
 * - remotehub-extension.json: `{ id, version }`, for that manifest
 *
 *   node scripts/pack-extension.mjs <built directory> <output directory>
 *
 * The signing key is REMOTEHUB_EXTENSION_KEY (a PEM file), else the
 * development key beside this package. It must be the key whose public half
 * the build wrote into the manifest (vite.extension.config.ts); the ZIP and
 * the CRX then have the same ID, and the script refuses anything else.
 */
import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { zipSync } from 'fflate';

const DEV_KEY = resolve(fileURLToPath(new URL('..', import.meta.url)), 'extension-dev-key.pem');

/**
 * Chromium's ID for a public key (SPKI, DER): the first 16 bytes of its
 * SHA-256, each hex digit shifted into the letters a to p.
 *
 * @param {Buffer} publicKey
 * @returns {string}
 */
export function extensionId(publicKey) {
	const hex = createHash('sha256').update(publicKey).digest().subarray(0, 16).toString('hex');
	return [...hex].map((digit) => String.fromCharCode(97 + parseInt(digit, 16))).join('');
}

/**
 * The files below `dir` in a ZIP archive, in a fixed order and with a fixed
 * time, so the same build gives the same bytes.
 *
 * @param {string} dir
 * @returns {Uint8Array}
 */
export function zip(dir) {
	/** @type {Record<string, Uint8Array>} */
	const files = {};
	const walk = (/** @type {string} */ at) => {
		for (const name of readdirSync(at).sort()) {
			const path = join(at, name);
			if (statSync(path).isDirectory()) walk(path);
			else files[relative(dir, path).split(sep).join('/')] = readFileSync(path);
		}
	};
	walk(dir);
	return zipSync(files, { level: 9, mtime: new Date('2000-01-01T00:00:00Z') });
}

/** A protobuf varint. */
function varint(/** @type {number} */ value) {
	const bytes = [];
	while (value > 0x7f) {
		bytes.push((value & 0x7f) | 0x80);
		value >>>= 7;
	}
	bytes.push(value);
	return Buffer.from(bytes);
}

/** A length-delimited protobuf field: tag, length, bytes. */
function field(/** @type {number} */ number, /** @type {Buffer} */ bytes) {
	return Buffer.concat([varint((number << 3) | 2), varint(bytes.length), bytes]);
}

/** A little-endian uint32. */
function uint32(/** @type {number} */ value) {
	const bytes = Buffer.alloc(4);
	bytes.writeUInt32LE(value);
	return bytes;
}

/**
 * The archive as a CRX3 file, signed with `privateKey` (RSA, SHA-256), as
 * Chromium's components/crx_file/crx3.proto describes it:
 *
 *     "Cr24" | version 3 | header size | CrxFileHeader | archive
 *
 * The header carries the public key and the signature over
 * "CRX3 SignedData\0", the size and bytes of `SignedData` (the ID), and
 * the archive.
 *
 * @param {Uint8Array} archive
 * @param {import('node:crypto').KeyObject} privateKey
 * @returns {Buffer}
 */
export function crx(archive, privateKey) {
	const publicKey = createPublicKey(privateKey).export({ type: 'spki', format: 'der' });
	const crxId = createHash('sha256').update(publicKey).digest().subarray(0, 16);
	const signedData = field(1, crxId);
	const signature = sign(
		'sha256',
		Buffer.concat([
			Buffer.from('CRX3 SignedData\x00', 'latin1'),
			uint32(signedData.length),
			signedData,
			archive
		]),
		privateKey
	);
	const header = Buffer.concat([
		// sha256_with_rsa: AsymmetricKeyProof { public_key, signature }
		field(2, Buffer.concat([field(1, publicKey), field(2, signature)])),
		// signed_header_data
		field(10000, signedData)
	]);
	return Buffer.concat([Buffer.from('Cr24'), uint32(3), uint32(header.length), header, archive]);
}

/**
 * Packs `built` into `out` with the key in the PEM file `keyPath`.
 *
 * @returns {{ id: string, version: string }}
 */
export function pack(/** @type {string} */ built, /** @type {string} */ out, keyPath = DEV_KEY) {
	const privateKey = createPrivateKey(readFileSync(keyPath));
	const publicKey = createPublicKey(privateKey).export({ type: 'spki', format: 'der' });
	const manifest = JSON.parse(readFileSync(join(built, 'manifest.json'), 'utf8'));
	if (manifest.key !== publicKey.toString('base64')) {
		throw new Error(`${keyPath} is not the key the build wrote into its manifest`);
	}
	const archive = zip(built);
	const info = { id: extensionId(publicKey), version: manifest.version };
	mkdirSync(out, { recursive: true });
	writeFileSync(join(out, 'remotehub-extension.zip'), archive);
	writeFileSync(join(out, 'remotehub-extension.crx'), crx(archive, privateKey));
	writeFileSync(join(out, 'remotehub-extension.json'), `${JSON.stringify(info)}\n`);
	return info;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
	const [built, out] = process.argv.slice(2);
	if (!built || !out) {
		console.error('usage: node scripts/pack-extension.mjs <built directory> <output directory>');
		process.exit(2);
	}
	const info = pack(built, out, process.env.REMOTEHUB_EXTENSION_KEY || DEV_KEY);
	console.log(`extension ${info.id} ${info.version} packed into ${out}`);
}
