/**
 * KeePass files for collections of shared credentials (#99, #190): an
 * export reveals every secret in the collection, each audited with the
 * purpose `export`; an import creates collections, credentials and their
 * files as a person would.
 */
import {
	allows,
	createCollection,
	createCredential,
	type CredentialInput,
	type Tree
} from '$lib/api/catalog';
import { attachmentUrl, reveal, uploadAttachment } from '$lib/api/reveal';
import { collectionPath, collectionsBelow } from '$lib/catalog/tree';
import type { KdbxEntry } from './kdbx';

export type ExportResult =
	| { ok: true; entries: KdbxEntry[] }
	| { ok: false; missing: string }
	| { ok: false; failed: string };

/** The collection's credentials as KeePass entries, their paths below it. */
export async function exportCollection(tree: Tree, collectionId: string): Promise<ExportResult> {
	const inside = collectionsBelow(tree, collectionId);
	const credentials = tree.credentials.filter((c) => inside.has(c.collection_id));
	// All or nothing: a file with gaps would pass for complete.
	const hidden = credentials.find((c) => !allows(c.role, 'reveal'));
	if (hidden) return { ok: false, missing: hidden.name };
	const entries: KdbxEntry[] = [];
	for (const credential of credentials) {
		const revealed = await reveal('credentials', credential.id, 'export');
		if (!revealed.ok) return { ok: false, failed: credential.name };
		const secret = revealed.data;
		const names = collectionPath(tree, credential.collection_id).map((c) => c.name);
		const start = collectionPath(tree, collectionId).length;
		const fields: KdbxEntry['fields'] = credential.fields
			.filter((field) => !field.protected)
			.map((field) => ({ name: field.name, value: field.value ?? '', protected: false }));
		for (const field of secret.fields ?? []) {
			fields.push({ name: field.name, value: field.value, protected: true });
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
			password: secret.password ?? '',
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

/** Creates the entries below `collectionId`, with a collection for each group. */
export async function importInto(
	tree: Tree,
	collectionId: string,
	entries: KdbxEntry[]
): Promise<ImportResult> {
	const collections = new Map<string, string>([['', collectionId]]);
	const result: ImportResult = { created: 0, failed: [] };
	const collectionOf = async (path: string[]): Promise<string | null> => {
		const key = path.join('\n');
		const known = collections.get(key);
		if (known) return known;
		const parent = await collectionOf(path.slice(0, -1));
		if (!parent) return null;
		const name = path[path.length - 1].trim() || '—';
		const existing = tree.collections.find((c) => c.parent_id === parent && c.name === name);
		let id = existing?.id;
		if (!id) {
			const made = await createCollection(parent, name);
			if (!made.ok) return null;
			id = made.data.id;
		}
		collections.set(key, id);
		return id;
	};
	for (const entry of entries) {
		const collection = await collectionOf(entry.path);
		const title = entry.title.trim() || entry.username.trim() || entry.url.trim() || '—';
		if (!collection) {
			result.failed.push(title);
			continue;
		}
		const input: CredentialInput = {
			collection_id: collection,
			name: title.slice(0, 200),
			username: entry.username.slice(0, 256),
			domain: '',
			password: entry.password,
			url: entry.url.slice(0, 2000),
			notes: entry.notes.slice(0, 10000),
			icon: entry.icon >= 0 && entry.icon <= 68 ? entry.icon : 0,
			fields: usableFields(entry.fields)
		};
		// A name taken in the collection gets a number, as a person would do.
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
