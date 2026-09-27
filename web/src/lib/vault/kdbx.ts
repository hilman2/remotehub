/**
 * KeePass files (KDBX 3 and 4) in the browser (#99), with kdbxweb and
 * Argon2 from hash-wasm. Personal entries never reach the server in plain
 * text, so they are read and written here; shared ones take the same way,
 * their secrets revealed one by one and audited.
 */
import { argon2d, argon2id } from 'hash-wasm';
import * as kdbxweb from 'kdbxweb';

/** An entry on its way into or out of a KeePass file. */
export interface KdbxEntry {
	/** The groups it lies in, below the file's top group. */
	path: string[];
	title: string;
	username: string;
	password: string;
	url: string;
	notes: string;
	/** KeePass' icon number. */
	icon: number;
	fields: { name: string; value: string; protected: boolean }[];
	files: { name: string; data: Uint8Array<ArrayBuffer> }[];
	tags?: string[];
	/** `YYYY-MM-DD`: when it runs out; none: never. */
	expires?: string | null;
	/** Its one-time password: an `otpauth://` link or a base32 secret. */
	totp?: string;
}

/** The fields every KeePass entry has; the rest are custom fields. */
const STANDARD = ['Title', 'UserName', 'Password', 'URL', 'Notes'];

/**
 * The one-time password of an entry's custom fields, and the fields without
 * it. KeePassXC keeps an `otpauth://` link in `otp`; KeePass 2 keeps the
 * secret and its settings in `TimeOtp-*` fields.
 */
export function takeTotp(fields: KdbxEntry['fields']): {
	totp: string;
	fields: KdbxEntry['fields'];
} {
	const byName = (name: string) =>
		fields.find((field) => field.name.toLowerCase() === name.toLowerCase())?.value;
	const link = byName('otp');
	if (link) return { totp: link, fields: fields.filter((f) => f.name.toLowerCase() !== 'otp') };
	const secret = byName('TimeOtp-Secret-Base32');
	if (!secret) return { totp: '', fields };
	const query = new URLSearchParams({ secret });
	const algorithm = byName('TimeOtp-Algorithm');
	if (algorithm) query.set('algorithm', algorithm.replace('HMAC-', '').replace('-', ''));
	const digits = byName('TimeOtp-Length');
	if (digits) query.set('digits', digits);
	const period = byName('TimeOtp-Period');
	if (period) query.set('period', period);
	return {
		totp: `otpauth://totp/entry?${query}`,
		fields: fields.filter((f) => !f.name.startsWith('TimeOtp-'))
	};
}

/** A date as `YYYY-MM-DD` in the local calendar. */
const day = (date: Date) =>
	`${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;

let engineReady = false;

function engine() {
	if (engineReady) return;
	kdbxweb.CryptoEngine.setArgon2Impl(
		async (password, salt, memory, iterations, length, parallelism, type, version) => {
			// hash-wasm knows Argon2 1.3 only; 1.0 files are rare.
			if (version !== 0x13) throw new Error('Argon2 version 1.0 is not supported');
			const hash = type === 2 ? argon2id : argon2d;
			const bytes = await hash({
				password: new Uint8Array(password),
				salt: new Uint8Array(salt),
				memorySize: memory,
				iterations,
				hashLength: length,
				parallelism,
				outputType: 'binary'
			});
			return bytes.slice().buffer;
		}
	);
	engineReady = true;
}

function text(value: string | kdbxweb.ProtectedValue | undefined): string {
	if (value === undefined) return '';
	return typeof value === 'string' ? value : value.getText();
}

function bytes(value: kdbxweb.KdbxBinary | kdbxweb.KdbxBinaryWithHash): Uint8Array<ArrayBuffer> {
	const binary = 'value' in value ? value.value : value;
	if (binary instanceof kdbxweb.ProtectedValue) return new Uint8Array(binary.getBinary());
	return new Uint8Array(binary.slice(0));
}

/** The entries of a KeePass file, without those in its recycle bin. */
export async function readKdbx(file: ArrayBuffer, password: string): Promise<KdbxEntry[]> {
	engine();
	const credentials = new kdbxweb.KdbxCredentials(kdbxweb.ProtectedValue.fromString(password));
	const db = await kdbxweb.Kdbx.load(file, credentials);
	const bin = db.meta.recycleBinUuid;
	const entries: KdbxEntry[] = [];
	const walk = (group: kdbxweb.KdbxGroup, path: string[]) => {
		if (bin && group.uuid.equals(bin)) return;
		for (const entry of group.entries) {
			const fields: KdbxEntry['fields'] = [];
			for (const [name, value] of entry.fields) {
				if (STANDARD.includes(name)) continue;
				fields.push({
					name,
					value: text(value),
					protected: value instanceof kdbxweb.ProtectedValue
				});
			}
			const otp = takeTotp(fields);
			const expiry = entry.times.expires && entry.times.expiryTime;
			entries.push({
				path,
				title: text(entry.fields.get('Title')),
				username: text(entry.fields.get('UserName')),
				password: text(entry.fields.get('Password')),
				url: text(entry.fields.get('URL')),
				notes: text(entry.fields.get('Notes')),
				icon: entry.icon ?? 0,
				fields: otp.fields,
				files: [...entry.binaries].map(([name, value]) => ({ name, data: bytes(value) })),
				tags: [...(entry.tags ?? [])],
				expires: expiry ? day(expiry) : null,
				totp: otp.totp
			});
		}
		for (const child of group.groups) walk(child, [...path, child.name ?? '']);
	};
	walk(db.getDefaultGroup(), []);
	return entries;
}

/** A new KeePass file (KDBX 4, Argon2id) with `entries`, sealed with `password`. */
export async function writeKdbx(
	entries: KdbxEntry[],
	password: string,
	name: string
): Promise<ArrayBuffer> {
	engine();
	const credentials = new kdbxweb.KdbxCredentials(kdbxweb.ProtectedValue.fromString(password));
	const db = kdbxweb.Kdbx.create(credentials, name);
	db.setKdf(kdbxweb.Consts.KdfId.Argon2id);
	const groups = new Map<string, kdbxweb.KdbxGroup>([['', db.getDefaultGroup()]]);
	const groupOf = (path: string[]): kdbxweb.KdbxGroup => {
		const key = path.join('\n');
		const known = groups.get(key);
		if (known) return known;
		const group = db.createGroup(groupOf(path.slice(0, -1)), path[path.length - 1]);
		groups.set(key, group);
		return group;
	};
	for (const item of entries) {
		const entry = db.createEntry(groupOf(item.path));
		entry.fields.set('Title', item.title);
		entry.fields.set('UserName', item.username);
		entry.fields.set('Password', kdbxweb.ProtectedValue.fromString(item.password));
		entry.fields.set('URL', item.url);
		entry.fields.set('Notes', item.notes);
		entry.icon = item.icon;
		for (const field of item.fields) {
			entry.fields.set(
				field.name,
				field.protected ? kdbxweb.ProtectedValue.fromString(field.value) : field.value
			);
		}
		if (item.totp) entry.fields.set('otp', kdbxweb.ProtectedValue.fromString(item.totp));
		if (item.tags?.length) entry.tags = [...item.tags];
		if (item.expires) {
			entry.times.expires = true;
			entry.times.expiryTime = new Date(`${item.expires}T00:00:00`);
		}
		for (const file of item.files) {
			entry.binaries.set(file.name, await db.createBinary(file.data.slice().buffer));
		}
	}
	return db.save();
}
