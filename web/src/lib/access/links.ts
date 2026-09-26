/** Where the Access page (#178) shows what: its tab and detail live in the query. */
import { resolve } from '$app/paths';
import type { ResolvedPathname } from '$app/types';

export type AccessTab = 'users' | 'groups' | 'roles' | 'folders';

export interface AccessView {
	tab: AccessTab;
	/** A user's id, a group's SID or a folder's id: its detail. */
	user?: string;
	group?: string;
	folder?: string;
}

/**
 * The address of a view of the Access page. Resolved like `resolve()`'s
 * own, so links and `goto` take it as one of the app's addresses.
 */
export function accessHref(view: AccessView): ResolvedPathname {
	const query = new URLSearchParams({ tab: view.tab });
	if (view.user) query.set('user', view.user);
	if (view.group) query.set('group', view.group);
	if (view.folder) query.set('folder', view.folder);
	return `${resolve('/access')}?${query}` as ResolvedPathname;
}

/** The view a query names; unknown tabs are the users. */
export function accessView(query: URLSearchParams): AccessView {
	const tab = query.get('tab');
	return {
		tab: tab === 'groups' || tab === 'roles' || tab === 'folders' ? tab : 'users',
		user: query.get('user') ?? undefined,
		group: query.get('group') ?? undefined,
		folder: query.get('folder') ?? undefined
	};
}
