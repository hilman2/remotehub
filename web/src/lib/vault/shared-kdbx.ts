/**
 * KeePass files for shared folders (#99): an export reveals every secret
 * in the folder, each audited with the purpose `export`; an import creates
 * folders, credentials and their files as a person would.
 */
import {
	allows,
	createCredential,
	createFolder,
	type CredentialInput,
	type Tree
} from '$lib/api/catalog';
import { attachmentUrl, reveal, uploadAttachment } from '$lib/api/reveal';
import { pathTo } from '$lib/catalog/tree';
import type { KdbxEntry } from './kdbx';

/** A folder and everything below it. */
function subtree(tree: Tree, folderId: string): Set<string> {
	const inside = new Set([folderId]);
	let grew = true;
	while (grew) {
		grew = false;
		for (const folder of tree.folders) {
			if (folder.parent_id && inside.has(folder.parent_id) && !inside.has(folder.id)) {
				inside.add(folder.id);
				grew = true;
			}
		}
	}
	return inside;
}

export type ExportResult =
	| { ok: true; entries: KdbxEntry[] }
	| { ok: false; missing: string }
	| { ok: false; failed: string };

/** The folder's credentials as KeePass entries, their paths below it. */
export async function exportFolder(tree: Tree, folderId: string): Promise<ExportResult> {
	const inside = subtree(tree, folderId);
	const credentials = tree.credentials.filter((c) => inside.has(c.folder_id));
	// All or nothing: a file with gaps would pass for complete.
	const hidden = credentials.find((c) => !allows(c.role, 'reveal'));
	if (hidden) return { ok: false, missing: hidden.name };
	const entries: KdbxEntry[] = [];
	for (const credential of credentials) {
		const revealed = await reveal('credentials', credential.id, 'export');
		if (!revealed.ok) return { ok: false, failed: credential.name };
		const secret = revealed.data;
		const names = pathTo(tree, credential.folder_id).map((f) => f.name);
		const start = pathTo(tree, folderId).length;
		const fields: KdbxEntry['fields'] = credential.fields
			.filter((field) => !field.protected)
			.map((field) => ({ name: field.name, value: field.value ?? '', protected: false }));
		for (const field of secret.fields ?? []) {
			fields.push({ name: field.name, value: field.value, protected: true });
		}
		// KeePass has no place for a key: it goes into protected fields, and
		// its passphrase into the password.
		if (secret.private_key) {
			fields.push({ name: 'SSH private key', value: secret.private_key, protected: true });
		}
		if (secret.certificate) {
			fields.push({ name: 'SSH certificate', value: secret.certificate, protected: false });
		}
		const files: KdbxEntry['files'] = [];
		for (const attachment of credential.attachments) {
			const response = await fetch(attachmentUrl(credential.id, attachment.id));
			if (!response.ok) return { ok: false, failed: credential.name };
			files.push({ name: attachment.name, data: new Uint8Array(await response.arrayBuffer()) });
		}
		entries.push({
			path: names.slice(start),
			title: credential.name,
			username: credential.domain
				? `${credential.domain}\\${credential.username}`
				: credential.username,
			password: secret.password ?? secret.passphrase ?? '',
			url: credential.url,
			notes: credential.notes,
			icon: credential.icon,
			fields,
			files
		});
	}
	return { ok: true, entries };
}

/** What an import did, and the entries it could not take. */
export interface ImportResult {
	created: number;
	failed: string[];
}

/** Custom fields remotehub takes: unique, plain names of up to 100 characters. */
function usableFields(fields: KdbxEntry['fields']): NonNullable<CredentialInput['fields']> {
	const seen = new Set<string>();
	return fields
		.map((field) => ({ ...field, name: field.name.trim() }))
		.filter((field) => {
			const ok =
				field.name &&
				field.name.length <= 100 &&
				!field.name.includes('\n') &&
				!seen.has(field.name);
			seen.add(field.name);
			return ok;
		})
		.map((field) => ({ name: field.name, protected: field.protected, value: field.value }));
}

/** Creates the entries below `folderId`, with a folder for each group. */
export async function importInto(
	tree: Tree,
	folderId: string,
	entries: KdbxEntry[]
): Promise<ImportResult> {
	const folders = new Map<string, string>([['', folderId]]);
	const result: ImportResult = { created: 0, failed: [] };
	const folderOf = async (path: string[]): Promise<string | null> => {
		const key = path.join('\n');
		const known = folders.get(key);
		if (known) return known;
		const parent = await folderOf(path.slice(0, -1));
		if (!parent) return null;
		const name = path[path.length - 1].trim() || '—';
		const existing = tree.folders.find((f) => f.parent_id === parent && f.name === name);
		let id = existing?.id;
		if (!id) {
			const made = await createFolder(parent, name);
			if (!made.ok) return null;
			id = made.data.id;
		}
		folders.set(key, id);
		return id;
	};
	for (const entry of entries) {
		const folder = await folderOf(entry.path);
		const title = entry.title.trim() || entry.username.trim() || entry.url.trim() || '—';
		if (!folder) {
			result.failed.push(title);
			continue;
		}
		const input: CredentialInput = {
			folder_id: folder,
			name: title.slice(0, 200),
			username: entry.username.slice(0, 256),
			domain: '',
			kind: 'password',
			password: entry.password,
			url: entry.url.slice(0, 2000),
			notes: entry.notes.slice(0, 10000),
			icon: entry.icon >= 0 && entry.icon <= 68 ? entry.icon : 0,
			fields: usableFields(entry.fields)
		};
		// A name taken in the folder gets a number, as a person would do.
		let made = await createCredential(input);
		for (let n = 2; !made.ok && made.code === 'name_taken' && n < 100; n++) {
			made = await createCredential({ ...input, name: `${input.name} (${n})` });
		}
		if (!made.ok) {
			result.failed.push(title);
			continue;
		}
		for (const file of entry.files) {
			await uploadAttachment(made.data.id, new File([file.data], file.name));
		}
		result.created += 1;
	}
	return result;
}
