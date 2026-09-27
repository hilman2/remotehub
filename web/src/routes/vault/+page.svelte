<script lang="ts">
	/**
	 * The vault (#190): the personal vault and the shared collections side
	 * by side. The sidebar chooses where to look, the list shows what is
	 * there, the pane beside it the entry chosen. Shared collections work
	 * without the personal vault; it opens in this browser only.
	 */
	import CircleAlert from '@lucide/svelte/icons/circle-alert';
	import FileDown from '@lucide/svelte/icons/file-down';
	import FileUp from '@lucide/svelte/icons/file-up';
	import Fingerprint from '@lucide/svelte/icons/fingerprint';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import Library from '@lucide/svelte/icons/library';
	import Lock from '@lucide/svelte/icons/lock';
	import Paperclip from '@lucide/svelte/icons/paperclip';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Timer from '@lucide/svelte/icons/timer';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { page } from '$app/state';
	import {
		allows,
		createCredential,
		deleteCollection,
		deleteCredential,
		loadTree,
		updateCredential,
		type Collection,
		type Credential,
		type CredentialInput,
		type Role,
		type Tree
	} from '$lib/api/catalog';
	import { errorMessage, problemMessage } from '$lib/api/errors';
	import { createRequest } from '$lib/api/requests';
	import { loadPicks, savePick } from '$lib/api/search';
	import CredentialForm from '$lib/catalog/CredentialForm.svelte';
	import Grants from '$lib/catalog/Grants.svelte';
	import RequestForm from '$lib/catalog/RequestForm.svelte';
	import { collectionPlaces, collectionPath } from '$lib/catalog/tree';
	import Dialog from '$lib/components/Dialog.svelte';
	import SettingsMenu, { type MenuItem } from '$lib/components/SettingsMenu.svelte';
	import { formatLocale, getLocale } from '$lib/i18n';
	import { m } from '$lib/paraglide/messages';
	import { pickKey } from '$lib/search/catalog';
	import { queryKey, remember, type Pick } from '$lib/search/rank';
	import { session } from '$lib/session.svelte';
	import CollectionForm from '$lib/vault/CollectionForm.svelte';
	import FieldsEditor, { type EditedField } from '$lib/vault/FieldsEditor.svelte';
	import IconPicker from '$lib/vault/IconPicker.svelte';
	import { FOLDER_ICON, icon } from '$lib/vault/icons';
	import {
		counter,
		listed,
		outline,
		personalFolders,
		personalItems,
		sharedItems,
		type Item,
		type Kind,
		type Scope,
		type Sort
	} from '$lib/vault/items';
	import KdbxForm from '$lib/vault/KdbxForm.svelte';
	import { writeKdbx, type KdbxEntry } from '$lib/vault/kdbx';
	import PasswordInput from '$lib/vault/PasswordInput.svelte';
	import PersonalDetail from '$lib/vault/PersonalDetail.svelte';
	import SharedDetail from '$lib/vault/SharedDetail.svelte';
	import { exportCollection, importInto } from '$lib/vault/shared-kdbx';
	import { unlocked } from '$lib/vault/unlocked.svelte';
	import {
		addPasskey,
		asKdbx,
		coverForOrganisation,
		deleteEntry,
		deleteFile,
		loadVault,
		MAX_FILE,
		oneTimeRecovery,
		passkeysAvailable,
		readEntries,
		readFile,
		readPicks,
		removeUnlock,
		renewRecovery,
		resetVault,
		saveEntry,
		saveFile,
		savePassphrase,
		savePicks,
		setUp,
		unlockWithPasskey,
		unlockWithPassphrase,
		unlockWithRecovery,
		withHistory,
		type Entry,
		type EntryContent,
		type FileRef,
		type StoredVault,
		type UnlockKind
	} from '$lib/vault/vault';

	type Stage = 'loading' | 'setup' | 'recovery' | 'locked' | 'open';

	const MIN_PASSPHRASE = 12;
	const KIND_LABELS: Record<UnlockKind, () => string> = {
		passkey: m.vault_kind_passkey,
		passphrase: m.vault_kind_passphrase,
		recovery: m.vault_kind_recovery,
		organisation: m.vault_kind_organisation
	};
	const FILTERS: { kind: Kind; label: () => string }[] = [
		{ kind: 'totp', label: m.vault_kind_totp },
		{ kind: 'files', label: m.vault_kind_files }
	];
	const EMPTY: EntryContent = { title: '', username: '', password: '', url: '', notes: '' };
	const SORT_KEY = 'remotehub.vault.sort';

	// ---- The personal vault ----

	let stage = $state<Stage>('loading');
	let vault = $state<StoredVault | null>(null);
	// The vault key: in memory only, kept in `unlocked` while the page is
	// loaded, so it is still open after a visit to another page.
	let key: CryptoKey | null = unlocked.key;
	let entries = $state<Entry[]>([]);
	let error = $state<string | null>(null);
	let busy = $state(false);

	let passphrase = $state('');
	let passphraseAgain = $state('');
	let recoveryText = $state('');
	let useRecovery = $state(false);
	let shownRecovery = $state('');
	let passkeyLabel = $state('');
	/** After a one-time recovery key: the new passphrase comes next. */
	let mustRenew = $state(false);

	/** What the owner used; personal ones sealed in the vault (#81). */
	let picks = $state<Pick[]>([]);

	// ---- The shared collections ----

	let tree = $state<Tree | null>(null);
	/** Picks of shared credentials, kept on the server like the devices page's. */
	let sharedPicks = $state<Pick[]>([]);

	// ---- What is shown ----

	let query = $state('');
	let scope = $state<Scope>({ kind: 'all' });
	let sort = $state<Sort>(readSort());
	/** The key of the item chosen (Item.key). */
	let chosen = $state<string | null>(null);

	function readSort(): Sort {
		try {
			const stored = localStorage.getItem(SORT_KEY);
			if (stored === 'name' || stored === 'used' || stored === 'place') return stored;
		} catch {
			// Without storage, the default is fine.
		}
		return 'name';
	}

	$effect(() => {
		try {
			localStorage.setItem(SORT_KEY, sort);
		} catch {
			// A preference only.
		}
	});

	const folders = $derived(personalFolders(entries));
	const items = $derived([...(tree ? sharedItems(tree) : []), ...personalItems(entries)]);
	const allPicks = $derived([...picks, ...sharedPicks]);
	/** Picks fade with age; the page's lifetime is short enough to hold one time. */
	const now = Date.now();
	const searching = $derived(queryKey(query).length > 0);
	const shown = $derived(
		listed(items, {
			scope,
			sort,
			query,
			picks: allPicks,
			now,
			folders,
			collections: tree?.collections ?? [],
			locale: getLocale()
		})
	);
	const count = $derived(
		counter(items, { folders, collections: tree?.collections ?? [], picks: allPicks, now })
	);
	const current = $derived(items.find((item) => item.key === chosen) ?? null);
	const personalOutline = $derived(outline(folders, getLocale()));
	const collectionOutline = $derived(outline(tree?.collections ?? [], getLocale()));
	/** Entries that do not open with this key; they can only be deleted. */
	const unreadable = $derived(entries.filter((entry) => !entry.content));

	/** The collection or personal folder shown, if the scope is one. */
	const scopeCollection = $derived.by(() => {
		const here = scope;
		return here.kind === 'collection' ? tree?.collections.find((c) => c.id === here.id) : undefined;
	});
	const scopeFolder = $derived.by(() => {
		const here = scope;
		return here.kind === 'personal' && here.folder
			? folders.find((f) => f.id === here.folder)
			: undefined;
	});
	const personalScope = $derived(scope.kind === 'personal');
	/** The personal vault is shown but not open: its setup or unlock form takes the list's place. */
	const staging = $derived(personalScope && !searching && stage !== 'open');
	/** The collections new shared credentials may go into. */
	const places = $derived(tree ? collectionPlaces(tree, getLocale()) : []);

	const sameScope = (a: Scope, b: Scope) => JSON.stringify(a) === JSON.stringify(b);

	const scopeTitle = $derived.by(() => {
		if (searching) return m.vault_search_title();
		switch (scope.kind) {
			case 'all':
				return m.vault_all();
			case 'recent':
				return m.vault_recent();
			case 'filter':
				return FILTERS.find((f) => f.kind === (scope as { filter: Kind }).filter)?.label() ?? '';
			case 'personal':
				return scopeFolder?.name ?? m.vault_personal();
			case 'collection':
				return scopeCollection?.name ?? '';
		}
	});
	const scopeNote = $derived.by(() => {
		if (searching) return m.vault_search_note();
		if (scope.kind === 'personal') return m.vault_scope_personal_note();
		if (scope.kind === 'collection' && tree) {
			const above = collectionPath(tree, scope.id).slice(0, -1);
			return above.length > 0
				? m.vault_in_collection({ path: above.map((c) => c.name).join(' / ') })
				: m.vault_scope_shared_note();
		}
		return m.vault_scope_mixed_note();
	});

	// ---- Loading ----

	async function loadPersonal() {
		const result = await loadVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		vault = result.data;
		if (key && (await coverForOrganisation(key, vault))) {
			const again = await loadVault();
			if (again.ok) vault = again.data;
		}
		if (key) {
			[entries, picks] = await Promise.all([readEntries(key, vault), readPicks(key, vault)]);
			if (stage === 'loading') stage = 'open';
		} else stage = vault.unlocks.length === 0 ? 'setup' : 'locked';
	}

	async function loadShared() {
		const [result, picked] = await Promise.all([loadTree(), loadPicks()]);
		if (result.ok) tree = result.data;
		else error = errorMessage(result.code);
		if (picked.ok) sharedPicks = picked.data.filter((pick) => pick.key.startsWith('credential:'));
	}

	$effect(() => {
		loadPersonal();
		loadShared().then(() => {
			// A link from a device names the credential to show.
			const linked = page.url.searchParams.get('credential');
			const credential = tree?.credentials.find((c) => c.id === linked);
			if (credential) {
				scope = { kind: 'collection', id: credential.collection_id };
				chosen = sharedItems(tree!).find((item) => item.id === credential.id)?.key ?? null;
			}
		});
	});

	// ---- Choosing ----

	function pick(next: Scope) {
		scope = next;
		query = '';
	}

	/** Chooses an item and remembers the pick for the current query. */
	function choose(item: Item) {
		chosen = item.key;
		used(item);
	}

	function used(item: Item) {
		if (item.source === 'personal') {
			if (!key) return;
			picks = remember(picks, item.key, query, Date.now());
			// A preference: if it is not stored, the vault still works.
			savePicks(key, picks);
		} else {
			sharedPicks = remember(sharedPicks, item.key, query, Date.now());
			savePick(item.key, queryKey(query));
		}
	}

	// ---- Setting up and unlocking the personal vault ----

	function passphraseProblem(): string | null {
		if (passphrase.length < MIN_PASSPHRASE)
			return m.vault_passphrase_short({ count: MIN_PASSPHRASE });
		if (passphrase !== passphraseAgain) return m.vault_passphrase_mismatch();
		return null;
	}

	async function create(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error) return;
		busy = true;
		const result = await setUp(passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		key = unlocked.key = result.key;
		shownRecovery = result.recovery;
		passphrase = passphraseAgain = '';
		stage = 'recovery';
	}

	async function opened(opening: CryptoKey | null) {
		if (!opening || !vault) {
			error = m.vault_wrong();
			return;
		}
		key = unlocked.key = opening;
		error = null;
		passphrase = recoveryText = '';
		// A one-time key from a recovery (#95) is replaced at once, and the
		// forgotten passphrase with it.
		if (useRecovery && oneTimeRecovery(vault)) {
			mustRenew = true;
			useRecovery = false;
			await newRecovery();
			return;
		}
		// Reads the entries, and wraps the key for the organisation if needed.
		await loadPersonal();
		stage = 'open';
	}

	async function unlock(event: SubmitEvent) {
		event.preventDefault();
		if (!vault) return;
		busy = true;
		const opening = useRecovery
			? await unlockWithRecovery(vault, recoveryText)
			: await unlockWithPassphrase(vault, passphrase);
		busy = false;
		await opened(opening);
	}

	async function unlockPasskey() {
		if (!vault) return;
		try {
			await opened(await unlockWithPasskey(vault));
		} catch {
			error = m.vault_wrong();
		}
	}

	function lock() {
		key = unlocked.key = null;
		entries = [];
		picks = [];
		if (current?.source === 'personal') chosen = null;
		stage = 'locked';
	}

	async function afterRecovery() {
		await loadPersonal();
		if (key && vault) entries = await readEntries(key, vault);
		stage = 'open';
		if (mustRenew) {
			mustRenew = false;
			show({ type: 'passphrase' });
		}
	}

	// ---- Dialogs ----

	type Open =
		| { type: 'entry'; id: string | null; content: EntryContent }
		| { type: 'folder' }
		| { type: 'credential'; credential: Credential | null; collection: string }
		| { type: 'collection'; collection: Collection | null; parent: string | null }
		| { type: 'delete'; credential: Credential }
		| { type: 'request'; credential: Credential; role: Role }
		| { type: 'kdbx'; mode: 'import' | 'export'; collection: Collection | null }
		| { type: 'unlocks' }
		| { type: 'passphrase' }
		| { type: 'reset' };

	let open = $state<Open | null>(null);
	let dialogOpen = $state(false);
	let kdbxResult = $state<string | null>(null);
	let requested = $state<string | null>(null);

	function show(next: Open) {
		error = null;
		kdbxResult = null;
		open = next;
		dialogOpen = true;
	}

	function close() {
		dialogOpen = false;
		error = null;
	}

	const dialogTitle = $derived.by(() => {
		switch (open?.type) {
			case 'entry':
				return open.id ? m.catalog_edit() : m.vault_new_entry();
			case 'folder':
				return m.vault_new_folder();
			case 'credential':
				return open.credential ? m.catalog_edit() : m.catalog_new_credential();
			case 'collection':
				return open.collection ? m.vault_manage_collection() : m.vault_new_collection();
			case 'delete':
				return m.catalog_delete();
			case 'request':
				return m.request_title({ name: open.credential.name });
			case 'kdbx':
				return open.mode === 'import' ? m.kdbx_import() : m.kdbx_export();
			case 'unlocks':
				return m.vault_unlocks_title();
			case 'passphrase':
				return m.vault_change_passphrase();
			case 'reset':
				return m.vault_reset();
			default:
				return '';
		}
	});

	/** "New" makes what the place shown holds: a personal entry, or a shared one. */
	function newEntry() {
		const shared = scope.kind === 'collection' || (scope.kind !== 'personal' && stage !== 'open');
		if (shared) {
			const here = scopeCollection && places.some((p) => p.id === scopeCollection.id);
			show({
				type: 'credential',
				credential: null,
				collection: (here ? scopeCollection?.id : places[0]?.id) ?? ''
			});
		} else editEntry(null);
	}
	const mayCreate = $derived(
		scope.kind === 'collection'
			? allows(scopeCollection?.role ?? null, 'edit')
			: scope.kind === 'personal'
				? stage === 'open'
				: stage === 'open' || places.length > 0
	);

	// ---- Personal entries ----

	/** The custom fields of the entry being edited. */
	let editedFields = $state<EditedField[]>([]);
	/** Files chosen in the editor, sealed and stored on save. */
	let pendingFiles = $state<File[]>([]);
	/** Files removed in the editor, deleted once the entry is saved. */
	let removedFiles = $state<string[]>([]);
	let folderName = $state('');

	/** Personal folders with their path, for choosing where an entry goes. */
	const folderPlaces = $derived(
		personalOutline.map((node) => {
			const names: string[] = [];
			let at = folders.find((f) => f.id === node.id);
			while (at && names.length < 20) {
				names.unshift(at.name);
				at = folders.find((f) => f.id === at?.parent_id);
			}
			return { id: node.id, path: names.join(' / ') };
		})
	);

	function editEntry(entry: Entry | null) {
		// Entries from before #98 have neither folder nor icon.
		const here = scope.kind === 'personal' ? scope.folder : null;
		const content = { ...(entry?.content ?? { ...EMPTY, parent: here }) };
		content.parent ??= null;
		content.icon ??= 0;
		editedFields = (entry?.content?.fields ?? []).map((field) => ({ ...field }));
		pendingFiles = [];
		removedFiles = [];
		show({ type: 'entry', id: entry?.id ?? null, content });
	}

	function addFiles(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const picked = [...(input.files ?? [])];
		input.value = '';
		if (picked.some((file) => file.size > MAX_FILE)) {
			error = m.vault_file_too_large({ megabytes: MAX_FILE / 1024 / 1024 });
			return;
		}
		pendingFiles = [...pendingFiles, ...picked];
	}

	async function saveEditedEntry(event: SubmitEvent) {
		event.preventDefault();
		if (!key || open?.type !== 'entry') return;
		const editing = open;
		busy = true;
		const added: FileRef[] = [];
		for (const file of pendingFiles) {
			const saved = await saveFile(key, file);
			if (!saved) {
				busy = false;
				error = errorMessage('internal');
				return;
			}
			added.push(saved);
		}
		let content: EntryContent = {
			...editing.content,
			fields: editedFields.map(({ name, value, protected: hidden }) => ({
				name: name.trim(),
				value,
				protected: hidden
			})),
			attachments: [
				...(editing.content.attachments ?? []).filter((f) => !removedFiles.includes(f.id)),
				...added
			]
		};
		const before = entries.find((entry) => entry.id === editing.id)?.content;
		if (before) content = withHistory(before, content);
		const result = await saveEntry(key, editing.id, content);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		for (const id of removedFiles) await deleteFile(id);
		close();
		await loadPersonal();
		chosen = result.id ?? editing.id;
	}

	async function removeEntry(id: string) {
		const result = await deleteEntry(id);
		if (!result.ok) error = errorMessage(result.code);
		else {
			close();
			if (chosen === id) chosen = null;
		}
		await loadPersonal();
	}

	async function addFolder(event: SubmitEvent) {
		event.preventDefault();
		if (!key) return;
		const parent = scope.kind === 'personal' ? scope.folder : null;
		const result = await saveEntry(key, null, {
			...EMPTY,
			kind: 'folder',
			parent,
			title: folderName.trim(),
			icon: FOLDER_ICON
		});
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		folderName = '';
		close();
		await loadPersonal();
		if (result.id) pick({ kind: 'personal', folder: result.id });
	}

	/** Only an empty folder goes. */
	const emptyFolder = (id: string) => !entries.some((entry) => entry.content?.parent === id);

	async function removeFolder(id: string) {
		const parent = folders.find((f) => f.id === id)?.parent_id ?? null;
		await removeEntry(id);
		pick({ kind: 'personal', folder: parent });
	}

	/** Opens a file in the browser and hands it over as a download. */
	async function download(ref: FileRef) {
		if (!key) return;
		const blob = await readFile(key, ref);
		if (!blob) {
			error = errorMessage('not_found');
			return;
		}
		hand(blob, ref.name);
	}

	function hand(blob: Blob, name: string) {
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.href = url;
		link.download = name;
		link.click();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
	}

	// ---- Shared credentials and collections ----

	async function run<T>(
		change: Promise<
			{ ok: true; data: T } | { ok: false; code: string; params?: Record<string, unknown> }
		>,
		after?: (data: T) => void
	) {
		const result = await change;
		if (!result.ok) {
			error = problemMessage(result);
			return;
		}
		close();
		after?.(result.data);
		await loadShared();
	}

	function saveCredential(input: CredentialInput) {
		if (open?.type !== 'credential') return;
		const target = open.credential;
		run(target ? updateCredential(target.id, input) : createCredential(input), (data) => {
			const id = target?.id ?? (data as { id?: string } | undefined)?.id;
			if (id) chosen = pickKey('credential', id);
		});
	}

	function removeCredential() {
		if (open?.type !== 'delete') return;
		const id = open.credential.id;
		run(deleteCredential(id), () => {
			if (current?.id === id) chosen = null;
		});
	}

	function sendRequest(role: Role, minutes: number, reason: string) {
		if (open?.type !== 'request') return;
		const id = open.credential.id;
		run(createRequest({ kind: 'credential', id }, role, minutes, reason), () => (requested = id));
	}

	async function collectionSaved(id: string) {
		close();
		await loadShared();
		if (id) pick({ kind: 'collection', id });
	}

	function removeCollection(collection: Collection) {
		run(deleteCollection(collection.id), () => pick({ kind: 'all' }));
	}

	/** Behind the gear of a collection: managing it, and KeePass files. */
	function collectionMenu(collection: Collection): MenuItem[] {
		return [
			...(allows(collection.role, 'manage')
				? [
						{
							label: m.vault_manage_collection(),
							icon: ShieldCheck,
							onselect: () => show({ type: 'collection', collection, parent: null })
						},
						{
							label: m.vault_new_subcollection(),
							icon: Plus,
							onselect: () => show({ type: 'collection', collection: null, parent: collection.id })
						}
					]
				: []),
			...(allows(collection.role, 'edit')
				? [
						{
							label: m.kdbx_import(),
							icon: FileUp,
							onselect: () => show({ type: 'kdbx', mode: 'import', collection })
						}
					]
				: []),
			...(allows(collection.role, 'reveal')
				? [
						{
							label: m.kdbx_export(),
							icon: FileDown,
							onselect: () => show({ type: 'kdbx', mode: 'export', collection })
						}
					]
				: [])
		];
	}

	/** Behind the gear of the personal vault. */
	const personalMenu = $derived<MenuItem[]>(
		stage === 'open'
			? [
					{
						label: m.vault_new_folder(),
						icon: FolderPlus,
						onselect: () => show({ type: 'folder' })
					},
					...(scopeFolder && emptyFolder(scopeFolder.id)
						? [
								{
									label: m.vault_remove_folder(),
									icon: Trash2,
									danger: true,
									onselect: () => scopeFolder && removeFolder(scopeFolder.id)
								}
							]
						: []),
					{
						label: m.kdbx_import(),
						icon: FileUp,
						onselect: () => show({ type: 'kdbx', mode: 'import', collection: null })
					},
					{
						label: m.kdbx_export(),
						icon: FileDown,
						onselect: () => show({ type: 'kdbx', mode: 'export', collection: null })
					},
					{
						label: m.vault_unlocks_title(),
						icon: Fingerprint,
						onselect: () => show({ type: 'unlocks' })
					},
					{ label: m.vault_lock(), icon: Lock, onselect: lock }
				]
			: []
	);

	// ---- KeePass files ----

	async function importKdbx(imported: KdbxEntry[]) {
		if (open?.type !== 'kdbx') return;
		const into = open.collection;
		busy = true;
		error = null;
		if (into && tree) {
			const result = await importInto(tree, into.id, imported);
			kdbxResult = m.kdbx_imported({ count: result.created });
			if (result.failed.length > 0)
				error = m.kdbx_not_imported({ names: result.failed.join(', ') });
			await loadShared();
		} else if (key) {
			kdbxResult = m.kdbx_imported({ count: await importPersonal(key, imported) });
			await loadPersonal();
		}
		busy = false;
	}

	/** Takes a KeePass file's entries into the personal folder shown (#99). */
	async function importPersonal(opened: CryptoKey, imported: KdbxEntry[]): Promise<number> {
		const top = scope.kind === 'personal' ? scope.folder : null;
		const made: Record<string, string | null> = { '': top };
		const folderOf = async (path: string[]): Promise<string | null> => {
			const at = path.join('\n');
			if (at in made) return made[at];
			const parent = await folderOf(path.slice(0, -1));
			const saved = await saveEntry(opened, null, {
				...EMPTY,
				kind: 'folder',
				parent,
				title: path[path.length - 1],
				icon: FOLDER_ICON
			});
			const id = saved.ok ? saved.id : parent;
			made[at] = id;
			return id;
		};
		let created = 0;
		for (const item of imported) {
			const attachments: FileRef[] = [];
			for (const file of item.files) {
				const saved = await saveFile(opened, new File([file.data], file.name));
				if (saved) attachments.push(saved);
			}
			const saved = await saveEntry(opened, null, {
				title: item.title,
				username: item.username,
				password: item.password,
				url: item.url,
				notes: item.notes,
				icon: item.icon,
				fields: item.fields,
				attachments,
				parent: await folderOf(item.path)
			});
			if (saved.ok) created += 1;
		}
		return created;
	}

	async function exportKdbx(password: string) {
		if (open?.type !== 'kdbx') return;
		const from = open.collection;
		busy = true;
		error = null;
		if (from && tree) {
			const result = await exportCollection(tree, from.id);
			if (!result.ok) {
				busy = false;
				error =
					'missing' in result
						? m.kdbx_missing_reveal({ name: result.missing })
						: m.kdbx_export_failed({ name: result.failed });
				return;
			}
			hand(new Blob([await writeKdbx(result.entries, password, from.name)]), `${from.name}.kdbx`);
			kdbxResult = m.kdbx_exported({ count: result.entries.length });
		} else if (key) {
			const opened = key;
			const out = await asKdbx(entries, (ref) => readFile(opened, ref));
			hand(new Blob([await writeKdbx(out, password, m.vault_title())]), 'remotehub-vault.kdbx');
			kdbxResult = m.kdbx_exported({ count: out.length });
		}
		busy = false;
	}

	// ---- Ways to unlock ----

	async function newPasskey() {
		if (!key || !session.user) return;
		error = null;
		try {
			const result = await addPasskey(key, session.user.username, passkeyLabel.trim());
			if (!result.ok) {
				error =
					result.code === 'prf_unsupported' ? m.vault_prf_unsupported() : errorMessage(result.code);
				return;
			}
		} catch {
			error = m.vault_prf_unsupported();
			return;
		}
		passkeyLabel = '';
		await loadPersonal();
	}

	async function changePassphrase(event: SubmitEvent) {
		event.preventDefault();
		error = passphraseProblem();
		if (error || !key) return;
		busy = true;
		const result = await savePassphrase(key, passphrase);
		busy = false;
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		passphrase = passphraseAgain = '';
		close();
		await loadPersonal();
	}

	async function newRecovery() {
		if (!key) return;
		const result = await renewRecovery(key);
		if (!result.ok) {
			error = errorMessage(result.code ?? 'internal');
			return;
		}
		close();
		shownRecovery = result.recovery;
		stage = 'recovery';
	}

	async function dropUnlock(id: string) {
		const result = await removeUnlock(id);
		error = result.ok ? null : errorMessage(result.code);
		await loadPersonal();
	}

	async function startOver() {
		const result = await resetVault();
		if (!result.ok) {
			error = errorMessage(result.code);
			return;
		}
		close();
		key = unlocked.key = null;
		entries = [];
		stage = 'setup';
		await loadPersonal();
	}

	const field = 'mt-1 w-full rounded-lg border border-line bg-page px-3 py-2';
	const label = 'mt-3 block text-sm font-medium';
	const button =
		'inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm hover:bg-surface-2';
	const primary =
		'inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink disabled:opacity-50';
	const navItem =
		'flex w-full min-w-0 items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-surface-2 aria-[current=true]:bg-surface-2 aria-[current=true]:font-semibold';
	const kilobytes = new Intl.NumberFormat(formatLocale(), { style: 'unit', unit: 'kilobyte' });
	const size = (bytes: number) => kilobytes.format(Math.max(1, Math.round(bytes / 1024)));
</script>

{#snippet passphraseFields()}
	<label class={label} for="vault-passphrase">{m.field_passphrase()}</label>
	<input
		id="vault-passphrase"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphrase}
	/>
	<label class={label} for="vault-passphrase-again">{m.field_passphrase_again()}</label>
	<input
		id="vault-passphrase-again"
		type="password"
		class={field}
		required
		autocomplete="new-password"
		bind:value={passphraseAgain}
	/>
{/snippet}

{#snippet problem()}
	{#if error}
		<p class="mt-4 flex items-center gap-2 text-sm" role="alert">
			<CircleAlert size={16} class="shrink-0 text-critical" aria-hidden="true" />
			{error}
		</p>
	{/if}
{/snippet}

{#snippet scopeButton(target: Scope, text: string, depth: number, glyph: typeof Lock | null)}
	{@const Glyph = glyph}
	<button
		type="button"
		class={navItem}
		style:padding-left="{0.625 + depth * 1.1}rem"
		aria-current={!searching && sameScope(scope, target)}
		onclick={() => pick(target)}
	>
		{#if Glyph}<Glyph size={15} class="shrink-0 text-ink-3" aria-hidden="true" />{/if}
		<span class="flex-1 truncate">{text}</span>
		<span class="text-xs text-ink-3 tabular-nums">{count(target)}</span>
	</button>
{/snippet}

<!-- The personal vault's own stages, where its entries would be. -->
{#snippet personalStage()}
	{#if stage === 'setup'}
		<form class="m-4 rounded-card border border-line bg-surface p-5" onsubmit={create}>
			<h2 class="text-lg font-semibold">{m.vault_setup_title()}</h2>
			<p class="mt-1 text-sm text-ink-2">{m.vault_setup_hint()}</p>
			{@render passphraseFields()}
			<button type="submit" class="{primary} mt-5" disabled={busy}>{m.vault_set_up()}</button>
			{@render problem()}
		</form>
	{:else if stage === 'locked'}
		<form class="m-4 rounded-card border border-line bg-surface p-5" onsubmit={unlock}>
			<h2 class="text-lg font-semibold">{m.vault_unlock_title()}</h2>
			<p class="mt-1 text-sm text-ink-2">{m.vault_locked_hint()}</p>
			{#if useRecovery}
				<label class={label} for="vault-recovery">{m.field_recovery_key()}</label>
				<input
					id="vault-recovery"
					class="{field} font-mono"
					required
					autocomplete="off"
					spellcheck="false"
					bind:value={recoveryText}
				/>
			{:else}
				<label class={label} for="vault-unlock-passphrase">{m.field_passphrase()}</label>
				<input
					id="vault-unlock-passphrase"
					type="password"
					class={field}
					required
					autocomplete="current-password"
					bind:value={passphrase}
				/>
			{/if}
			<div class="mt-5 flex flex-wrap gap-2">
				<button type="submit" class={primary} disabled={busy}>{m.vault_unlock()}</button>
				{#if passkeysAvailable() && vault?.unlocks.some((u) => u.kind === 'passkey')}
					<button type="button" class={button} onclick={unlockPasskey}>
						<Fingerprint size={16} aria-hidden="true" />
						{m.vault_unlock_passkey()}
					</button>
				{/if}
				<button type="button" class={button} onclick={() => (useRecovery = !useRecovery)}>
					{useRecovery ? m.vault_use_passphrase() : m.vault_use_recovery()}
				</button>
			</div>
			<button
				type="button"
				class="mt-4 text-xs text-ink-3 underline"
				onclick={() => show({ type: 'reset' })}
			>
				{m.vault_reset()}
			</button>
			{@render problem()}
		</form>
	{/if}
{/snippet}

{#snippet row(item: Item)}
	{@const Icon = icon(item.icon)}
	<li>
		<button
			type="button"
			class="flex w-full min-w-0 items-center gap-3 rounded-xl border border-transparent px-3 py-2.5 text-left hover:bg-surface aria-[current=true]:border-accent aria-[current=true]:bg-surface"
			aria-current={chosen === item.key}
			data-testid={item.source === 'shared' ? 'shared-entry' : 'personal-entry'}
			onclick={() => choose(item)}
		>
			<span
				class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface-2 text-warning"
				aria-hidden="true"
			>
				<Icon size={17} />
			</span>
			<span class="min-w-0 flex-1">
				<span class="block truncate font-medium">{item.title}</span>
				<span class="block truncate text-xs text-ink-2">
					{item.username}{item.url ? ` · ${item.url}` : ''}
				</span>
			</span>
			<span class="flex shrink-0 flex-col items-end gap-1">
				<span class="flex max-w-36 items-center gap-1 truncate text-xs text-ink-3">
					{#if item.source === 'personal'}
						<Lock size={11} aria-hidden="true" />
						<span class="truncate">{item.where || m.vault_personal()}</span>
					{:else}
						<Library size={11} aria-hidden="true" />
						<span class="truncate">{item.where}</span>
					{/if}
				</span>
				<span class="flex gap-1 text-ink-3">
					{#if item.kinds.includes('totp')}
						<Timer size={13} aria-label={m.vault_kind_totp()} />
					{/if}
					{#if item.kinds.includes('files')}
						<Paperclip size={13} aria-label={m.vault_kind_files()} />
					{/if}
				</span>
			</span>
		</button>
	</li>
{/snippet}

<div class="flex min-h-0 flex-1 flex-col overflow-y-auto lg:flex-row lg:overflow-hidden">
	<aside
		class="flex shrink-0 flex-col gap-5 border-b border-line bg-sunken px-3 py-5 lg:w-72 lg:overflow-y-auto lg:border-r lg:border-b-0"
	>
		<h1 class="px-2 eyebrow">{m.nav_vault()}</h1>
		<label class="relative block">
			<span class="sr-only">{m.vault_search()}</span>
			<Search size={16} class="absolute top-3 left-3 text-ink-3" aria-hidden="true" />
			<input
				type="search"
				class="h-10 w-full rounded-xl border border-line-strong bg-page pr-3 pl-9 text-sm"
				placeholder={m.vault_search()}
				bind:value={query}
			/>
		</label>

		<nav aria-label={m.nav_vault()} class="flex flex-col gap-5">
			<div class="flex flex-col gap-0.5">
				{@render scopeButton({ kind: 'all' }, m.vault_all(), 0, null)}
				{@render scopeButton({ kind: 'recent' }, m.vault_recent(), 0, null)}
			</div>

			<section class="flex flex-col gap-0.5" aria-labelledby="vault-personal">
				<div class="flex items-center gap-2 px-2 pb-1">
					<h2 id="vault-personal" class="eyebrow">{m.vault_personal()}</h2>
					{#if stage === 'locked'}
						<span class="inline-flex items-center gap-1 text-xs text-warning">
							<Lock size={12} aria-hidden="true" />
							{m.vault_locked()}
						</span>
					{/if}
					<span class="ml-auto">
						<SettingsMenu label={m.vault_personal_settings()} items={personalMenu} />
					</span>
				</div>
				{@render scopeButton(
					{ kind: 'personal', folder: null },
					m.vault_personal_all(),
					0,
					stage === 'open' ? FolderIcon : Lock
				)}
				{#each personalOutline as node (node.id)}
					{@render scopeButton(
						{ kind: 'personal', folder: node.id },
						node.name,
						node.depth + 1,
						FolderIcon
					)}
				{/each}
			</section>

			<section class="flex flex-col gap-0.5" aria-labelledby="vault-collections">
				<div class="flex items-center gap-2 px-2 pb-1">
					<h2 id="vault-collections" class="eyebrow">{m.vault_collections()}</h2>
					{#if tree?.may_create_top_level}
						<button
							type="button"
							class="ml-auto flex size-7 items-center justify-center rounded-lg text-ink-2 hover:bg-surface-2 hover:text-ink"
							title={m.vault_new_collection()}
							onclick={() => show({ type: 'collection', collection: null, parent: null })}
						>
							<Plus size={15} aria-hidden="true" />
							<span class="sr-only">{m.vault_new_collection()}</span>
						</button>
					{/if}
				</div>
				{#each collectionOutline as node (node.id)}
					{@const collection = tree?.collections.find((c) => c.id === node.id)}
					<div class="group flex items-center">
						{@render scopeButton(
							{ kind: 'collection', id: node.id },
							node.name,
							node.depth,
							Library
						)}
						{#if collection}
							<!-- Only where the pointer or focus is, and on the collection shown. -->
							<span
								class="shrink-0 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 data-[shown=true]:opacity-100"
								data-shown={scope.kind === 'collection' && scope.id === node.id}
							>
								<SettingsMenu
									label={m.vault_collection_settings({ name: node.name })}
									items={collectionMenu(collection)}
								/>
							</span>
						{/if}
					</div>
				{:else}
					<p class="px-2.5 text-sm text-ink-3">{m.vault_no_collections()}</p>
				{/each}
			</section>

			<section class="flex flex-col gap-0.5" aria-labelledby="vault-kinds">
				<h2 id="vault-kinds" class="px-2 pb-1 eyebrow">{m.vault_kinds()}</h2>
				{#each FILTERS as filter (filter.kind)}
					{@render scopeButton({ kind: 'filter', filter: filter.kind }, filter.label(), 0, null)}
				{/each}
			</section>
		</nav>
	</aside>

	{#if stage === 'recovery'}
		<section class="flex-1 p-6 lg:overflow-y-auto">
			<div class="max-w-md rounded-card border border-line bg-surface p-6">
				<h2 class="text-lg font-semibold">{m.vault_recovery_title()}</h2>
				{#if mustRenew}
					<p class="mt-1 text-sm" role="status">{m.vault_recovered_hint()}</p>
				{/if}
				<p class="mt-1 text-sm text-ink-2">{m.vault_recovery_hint()}</p>
				<p
					class="mt-4 rounded-lg bg-surface-2 p-3 font-mono text-lg break-all"
					data-testid="recovery-key"
				>
					{shownRecovery}
				</p>
				<button type="button" class="{primary} mt-5" onclick={afterRecovery}>
					{m.vault_recovery_saved()}
				</button>
			</div>
		</section>
	{:else}
		<section
			class="flex shrink-0 flex-col border-b border-line bg-page lg:w-[26rem] lg:border-r lg:border-b-0"
			aria-labelledby="vault-scope"
		>
			<div class="flex flex-col gap-3 border-b border-line px-4 pt-5 pb-3">
				<div class="flex items-start gap-3">
					<div class="min-w-0 flex-1">
						<h2 id="vault-scope" class="truncate text-xl font-semibold">{scopeTitle}</h2>
						<p class="truncate text-sm text-ink-2">{scopeNote}</p>
					</div>
					{#if mayCreate && !(personalScope && stage !== 'open')}
						<button type="button" class={primary} onclick={newEntry}>
							<Plus size={16} aria-hidden="true" />
							{m.vault_new()}
						</button>
					{/if}
				</div>
				<div class="flex items-center gap-2 text-sm text-ink-2">
					{#if !searching && scope.kind !== 'recent'}
						<label class="flex items-center gap-2">
							{m.vault_sort()}
							<select
								class="rounded-md border border-line bg-surface px-2 py-1 text-ink"
								bind:value={sort}
							>
								<option value="name">{m.field_name()}</option>
								<option value="used">{m.vault_recent()}</option>
								<option value="place">{m.vault_sort_place()}</option>
							</select>
						</label>
					{/if}
					<span class="ml-auto tabular-nums">{m.vault_count({ count: shown.length })}</span>
				</div>
			</div>
			<!-- The setup and unlock forms show their own. -->
			{#if !dialogOpen && !staging}{@render problem()}{/if}
			<div class="flex-1 lg:overflow-y-auto">
				{#if staging}
					{@render personalStage()}
				{:else if (stage === 'locked' || stage === 'setup') && !searching && scope.kind === 'all'}
					<button
						type="button"
						class="mx-4 mt-3 flex w-[calc(100%-2rem)] items-center gap-2 rounded-lg border border-line bg-surface px-3 py-2 text-left text-sm text-ink-2 hover:bg-surface-2"
						onclick={() => pick({ kind: 'personal', folder: null })}
					>
						<Lock size={15} class="shrink-0 text-warning" aria-hidden="true" />
						{stage === 'locked' ? m.vault_locked_notice() : m.vault_setup_notice()}
					</button>
				{/if}
				{#if shown.length > 0}
					<ul class="flex flex-col gap-1 p-2">
						{#each shown as item (item.key)}
							{@render row(item)}
						{/each}
					</ul>
				{:else if !(personalScope && stage !== 'open')}
					<p class="px-4 py-6 text-sm text-ink-3">
						{searching ? m.catalog_no_match() : m.vault_empty()}
					</p>
				{/if}
				{#if personalScope && !searching && scope.kind === 'personal' && scope.folder === null && unreadable.length > 0}
					<ul class="flex flex-col gap-1 p-2">
						{#each unreadable as entry (entry.id)}
							<li class="flex items-center gap-3 rounded-xl px-3 py-2.5 text-sm">
								<CircleAlert size={16} class="shrink-0 text-critical" aria-hidden="true" />
								<span class="flex-1 text-ink-2">{m.vault_unreadable()}</span>
								<button type="button" class={button} onclick={() => removeEntry(entry.id)}>
									{m.catalog_delete()}
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
		</section>

		<section
			class="flex min-w-0 flex-1 flex-col px-5 py-7 sm:px-8 lg:overflow-y-auto"
			aria-label={m.vault_entry()}
			aria-live="polite"
		>
			{#if current?.source === 'personal' && current.entry.content}
				{@const entry = current.entry}
				{#key current.key}
					<PersonalDetail
						content={current.entry.content}
						where={current.where}
						onedit={() => editEntry(entry)}
						onused={() => used(current)}
						ondownload={download}
					/>
				{/key}
			{:else if current?.source === 'shared' && tree}
				{@const credential = current.credential}
				<SharedDetail
					{tree}
					{credential}
					onedit={() =>
						show({ type: 'credential', credential, collection: credential.collection_id })}
					ondelete={() => show({ type: 'delete', credential })}
					onrequest={() => show({ type: 'request', credential, role: credential.role })}
					onchange={loadShared}
				/>
				{#if requested === credential.id}
					<p class="mt-4 text-sm" role="status">{m.request_sent()}</p>
				{/if}
			{:else}
				<p class="m-auto text-sm text-ink-3">{m.vault_choose()}</p>
			{/if}
		</section>
	{/if}
</div>

<Dialog
	bind:open={dialogOpen}
	title={dialogTitle}
	wide={open?.type === 'collection' && !!open.collection}
>
	{#if dialogOpen && open?.type === 'entry'}
		{@const editing = open}
		<form onsubmit={saveEditedEntry}>
			<label class="block text-sm font-medium" for="entry-title">{m.field_title()}</label>
			<input
				id="entry-title"
				class={field}
				required
				maxlength="200"
				bind:value={editing.content.title}
			/>
			<label class={label} for="entry-username">{m.field_username()}</label>
			<input
				id="entry-username"
				class={field}
				autocomplete="off"
				bind:value={editing.content.username}
			/>
			<label class={label} for="entry-password">{m.field_password()}</label>
			<PasswordInput id="entry-password" bind:value={editing.content.password} />
			<label class={label} for="entry-url">{m.field_url()}</label>
			<input id="entry-url" class={field} bind:value={editing.content.url} />
			<label class={label} for="entry-notes">{m.field_notes()}</label>
			<textarea id="entry-notes" class={field} rows="3" bind:value={editing.content.notes}
			></textarea>
			<FieldsEditor id="entry" bind:fields={editedFields} />
			<fieldset class="mt-3">
				<legend class="text-sm font-medium">{m.vault_files()}</legend>
				<ul class="mt-1 text-sm">
					{#each (editing.content.attachments ?? []).filter((f) => !removedFiles.includes(f.id)) as file (file.id)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (removedFiles = [...removedFiles, file.id])}
							>
								{m.vault_remove()}
							</button>
						</li>
					{/each}
					{#each pendingFiles as file, index (index)}
						<li class="flex items-center gap-2">
							<span class="truncate">{file.name}</span>
							<span class="text-xs text-ink-3">{size(file.size)}</span>
							<button
								type="button"
								class="ml-auto rounded-md px-2 py-0.5 text-xs text-ink-3 hover:text-critical"
								onclick={() => (pendingFiles = pendingFiles.filter((_, i) => i !== index))}
							>
								{m.vault_remove()}
							</button>
						</li>
					{/each}
				</ul>
				<label
					class="mt-1 inline-block cursor-pointer rounded-md px-2 py-1 text-sm text-ink-2 underline hover:text-ink"
				>
					{m.vault_add_file()}
					<input type="file" class="sr-only" multiple onchange={addFiles} />
				</label>
			</fieldset>
			<label class={label} for="entry-folder">{m.vault_folder()}</label>
			<select id="entry-folder" class={field} bind:value={editing.content.parent}>
				<option value={null}>{m.vault_personal()}</option>
				{#each folderPlaces as place (place.id)}
					<option value={place.id}>{place.path}</option>
				{/each}
			</select>
			<label class={label} for="entry-icon">{m.vault_icon()}</label>
			<IconPicker id="entry-icon" bind:value={editing.content.icon} />
			<div class="mt-5 flex justify-end gap-2">
				{#if editing.id}
					<button
						type="button"
						class="mr-auto rounded-lg px-3 py-1.5 text-sm text-critical hover:bg-surface-2"
						onclick={() => editing.id && removeEntry(editing.id)}
					>
						{m.catalog_delete()}
					</button>
				{/if}
				<button
					type="button"
					class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
					onclick={close}
				>
					{m.action_cancel()}
				</button>
				<button type="submit" class={primary} disabled={busy}>{m.action_save()}</button>
			</div>
			{@render problem()}
		</form>
	{:else if dialogOpen && open?.type === 'folder'}
		<form onsubmit={addFolder}>
			<label class="block text-sm font-medium" for="vault-folder-name">{m.field_name()}</label>
			<input
				id="vault-folder-name"
				class={field}
				required
				maxlength="200"
				bind:value={folderName}
			/>
			<button type="submit" class="{primary} mt-5">{m.action_create()}</button>
			{@render problem()}
		</form>
	{:else if dialogOpen && open?.type === 'credential'}
		<CredentialForm
			collectionId={open.collection}
			{places}
			credential={open.credential}
			onsubmit={saveCredential}
			oncancel={close}
		/>
		{@render problem()}
	{:else if dialogOpen && open?.type === 'collection' && tree}
		{@const target = open}
		<CollectionForm
			{tree}
			collection={target.collection}
			parent={target.parent}
			onsaved={collectionSaved}
			oncancel={close}
		/>
		{#if target.collection}
			{@const collection = target.collection}
			<div class="mt-6 border-t border-line pt-5">
				<h3 class="font-semibold">{m.catalog_permissions()}</h3>
				<p class="mt-1 text-sm text-ink-2">{m.vault_collection_grants_hint()}</p>
				<div class="mt-3">
					<Grants kind="collection" id={collection.id} />
				</div>
			</div>
			<div class="mt-6 flex border-t border-line pt-4">
				<button
					type="button"
					class="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm text-critical hover:bg-surface-2"
					onclick={() => removeCollection(collection)}
				>
					<Trash2 size={15} aria-hidden="true" />
					{m.vault_delete_collection()}
				</button>
			</div>
			{@render problem()}
		{/if}
	{:else if dialogOpen && open?.type === 'delete'}
		<p class="text-sm">{m.catalog_delete_confirm({ name: open.credential.name })}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={close}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={removeCredential}
			>
				{m.catalog_delete()}
			</button>
		</div>
		{@render problem()}
	{:else if dialogOpen && open?.type === 'request'}
		<RequestForm held={open.role} onsubmit={sendRequest} oncancel={close} />
		{@render problem()}
	{:else if dialogOpen && open?.type === 'kdbx'}
		<KdbxForm mode={open.mode} {busy} onimport={importKdbx} onexport={exportKdbx} />
		{#if kdbxResult}
			<p class="mt-3 text-sm" role="status">{kdbxResult}</p>
		{/if}
		{@render problem()}
	{:else if dialogOpen && open?.type === 'unlocks'}
		<ul class="divide-y divide-line rounded-card border border-line bg-surface">
			{#each vault?.unlocks ?? [] as way (way.id)}
				<li class="flex items-center gap-3 px-4 py-2 text-sm">
					<span>{KIND_LABELS[way.kind]()}{way.label ? ` · ${way.label}` : ''}</span>
					{#if way.kind === 'passkey'}
						<button
							type="button"
							class="ml-auto rounded-md px-2 py-1 text-xs text-ink-3 hover:bg-surface-2 hover:text-critical"
							onclick={() => dropUnlock(way.id)}
						>
							{m.vault_remove()}
						</button>
					{/if}
				</li>
			{/each}
		</ul>
		{#if passkeysAvailable()}
			<div class="mt-4 flex flex-wrap items-end gap-2">
				<label class="text-sm">
					<span class="block font-medium">{m.vault_passkey_label()}</span>
					<input
						class="mt-1 rounded-lg border border-line bg-page px-3 py-1.5"
						bind:value={passkeyLabel}
					/>
				</label>
				<button type="button" class={button} onclick={newPasskey}>
					<Fingerprint size={16} aria-hidden="true" />
					{m.vault_add_passkey()}
				</button>
			</div>
		{/if}
		<div class="mt-4 flex flex-wrap gap-2">
			<button type="button" class={button} onclick={() => show({ type: 'passphrase' })}>
				<Pencil size={15} aria-hidden="true" />
				{m.vault_change_passphrase()}
			</button>
			<button type="button" class={button} onclick={newRecovery}>{m.vault_new_recovery()}</button>
		</div>
		{@render problem()}
	{:else if dialogOpen && open?.type === 'passphrase'}
		<form onsubmit={changePassphrase}>
			{@render passphraseFields()}
			<button type="submit" class="{primary} mt-5" disabled={busy}>{m.action_save()}</button>
			{@render problem()}
		</form>
	{:else if dialogOpen && open?.type === 'reset'}
		<p class="text-sm">{m.vault_reset_confirm()}</p>
		<div class="mt-5 flex justify-end gap-2">
			<button
				type="button"
				class="rounded-lg px-3 py-1.5 text-sm hover:bg-surface-2"
				onclick={close}
			>
				{m.action_cancel()}
			</button>
			<button
				type="button"
				class="rounded-lg bg-critical px-3 py-1.5 text-sm font-medium text-white"
				onclick={startOver}
			>
				{m.vault_reset()}
			</button>
		</div>
	{/if}
</Dialog>
