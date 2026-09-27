/**
 * What the vault does with an entry beyond showing it (#193): drafts for
 * the editor, and saving, moving and copying between the personal vault
 * and the shared collections. Both sides meet in the KeePass entry, which
 * an export and an import use as well.
 */
import type { Credential, CredentialInput } from '$lib/api/catalog';
import type { Draft } from './EntryEditor.svelte';
import { tagList } from './EntryEditor.svelte';
import type { KdbxEntry } from './kdbx';
import { personalTotp, type Item } from './items';
import {
	asKdbx,
	deleteEntry,
	deleteFile,
	readFile,
	saveEntry,
	saveFile,
	type Entry,
	type EntryContent,
	type FileRef
} from './vault';

/** Where a draft goes: a personal folder (null: the top) or a collection. */
export type Target = { side: 'personal'; folder: string | null } | { side: 'shared'; id: string };

export const placeOf = (target: Target) =>
	target.side === 'personal' ? `p:${target.folder ?? ''}` : `s:${target.id}`;

export function targetOf(place: string): Target {
	return place.startsWith('s:')
		? { side: 'shared', id: place.slice(2) }
		: { side: 'personal', folder: place.slice(2) || null };
}

/** An empty draft for a new entry at `place`. */
export function newDraft(place: string): Draft {
	return {
		title: '',
		username: '',
		password: '',
		url: '',
		notes: '',
		icon: 0,
		fields: [],
		tags: '',
		expires: '',
		totp: '',
		removeTotp: false,
		place,
		newFiles: [],
		removedFiles: []
	};
}

/** A draft of an entry as it is. A shared entry's secrets stay on the server. */
export function draftOf(item: Item): Draft {
	const base = {
		...newDraft(''),
		title: item.title,
		username: item.username,
		url: item.url,
		notes: item.notes,
		icon: item.icon,
		tags: item.tags.join(', '),
		expires: item.expires ?? ''
	};
	if (item.source === 'personal') {
		const content = item.entry.content;
		return {
			...base,
			password: content?.password ?? '',
			fields: (content?.fields ?? []).map((field) => ({ ...field })),
			totp: content?.totp ?? '',
			place: placeOf({ side: 'personal', folder: item.folder })
		};
	}
	return {
		...base,
		fields: item.credential.fields.map((field) => ({
			name: field.name,
			value: field.value ?? '',
			protected: !!field.protected,
			stored: !!field.protected
		})),
		place: placeOf({ side: 'shared', id: item.collection })
	};
}

/**
 * The personal entry a draft makes. `before` is what it was; its files that
 * stay are kept, and `added` are the ones just sealed.
 */
export function contentOf(
	draft: Draft,
	folder: string | null,
	before: EntryContent | null,
	added: FileRef[]
): EntryContent {
	return {
		...(before ?? {}),
		title: draft.title.trim(),
		username: draft.username,
		password: draft.password,
		url: draft.url.trim(),
		notes: draft.notes,
		icon: draft.icon,
		parent: folder,
		fields: draft.fields.map(({ name, value, protected: hidden }) => ({
			name: name.trim(),
			value,
			protected: hidden
		})),
		attachments: [
			...(before?.attachments ?? []).filter((file) => !draft.removedFiles.includes(file.id)),
			...added
		],
		tags: tagList(draft.tags),
		expires: draft.expires || null,
		totp: draft.removeTotp ? '' : draft.totp.trim(),
		changed: new Date().toISOString(),
		deleted: before?.deleted ?? null
	};
}

/** The request a draft makes for a shared entry in `collection`. */
export function inputOf(draft: Draft, collection: string, isNew: boolean): CredentialInput {
	return {
		collection_id: collection,
		name: draft.title.trim(),
		username: draft.username,
		url: draft.url.trim(),
		notes: draft.notes,
		icon: draft.icon,
		// A stored protected field left empty keeps its value.
		fields: draft.fields.map((field) =>
			field.protected && field.stored && field.value === ''
				? { name: field.name.trim(), protected: true }
				: { name: field.name.trim(), protected: field.protected, value: field.value }
		),
		tags: tagList(draft.tags),
		expires_on: draft.expires || null,
		// Updating without a new password keeps the stored one.
		...(draft.password || isNew ? { password: draft.password } : {}),
		...(draft.removeTotp ? { totp: '' } : draft.totp.trim() ? { totp: draft.totp.trim() } : {})
	};
}

/** A shared entry as it is, into another collection; its secrets stay. */
export function inputFromCredential(credential: Credential, collection: string): CredentialInput {
	return {
		collection_id: collection,
		name: credential.name,
		username: credential.username,
		url: credential.url,
		notes: credential.notes,
		icon: credential.icon,
		fields: credential.fields.map((field) =>
			field.protected
				? { name: field.name, protected: true }
				: { name: field.name, protected: false, value: field.value ?? '' }
		),
		tags: credential.tags,
		expires_on: credential.expires_on
	};
}

/** A personal entry as a KeePass entry at no path, its files opened. */
export async function personalAsKdbx(key: CryptoKey, entry: Entry): Promise<KdbxEntry | null> {
	const [out] = await asKdbx(
		[{ ...entry, content: entry.content && { ...entry.content, parent: null } }],
		(ref) => readFile(key, ref)
	);
	if (!out) return null;
	return { ...out, path: [], totp: personalTotp(entry) };
}

/** Creates a personal entry from a KeePass entry in `folder`; its id, or null. */
export async function savePersonalKdbx(
	key: CryptoKey,
	entry: KdbxEntry,
	folder: string | null
): Promise<string | null> {
	const attachments: FileRef[] = [];
	for (const file of entry.files) {
		const saved = await saveFile(key, new File([file.data], file.name));
		if (saved) attachments.push(saved);
	}
	const saved = await saveEntry(key, null, {
		title: entry.title,
		username: entry.username,
		password: entry.password,
		url: entry.url,
		notes: entry.notes,
		icon: entry.icon,
		fields: entry.fields,
		attachments,
		parent: folder,
		tags: entry.tags ?? [],
		expires: entry.expires ?? null,
		totp: entry.totp ?? '',
		changed: new Date().toISOString()
	});
	return saved.ok ? (saved.id ?? null) : null;
}

/** Deletes a personal entry for good, with its files. */
export async function purgePersonal(entry: Entry): Promise<boolean> {
	const result = await deleteEntry(entry.id);
	if (!result.ok) return false;
	for (const file of entry.content?.attachments ?? []) await deleteFile(file.id);
	return true;
}
