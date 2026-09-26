/** Words for the Access page (#178). */
import type { Role } from '$lib/api/catalog';
import type { UserKind } from '$lib/api/users';
import { m } from '$lib/paraglide/messages';
import type { Role as SiteRole } from '$lib/session.svelte';

export const USER_KIND_LABELS: Record<UserKind, () => string> = {
	directory: m.users_kind_directory,
	local: m.users_kind_local,
	break_glass: m.users_kind_break_glass
};

/** Roles for remotehub itself. */
export const SITE_ROLE_LABELS: Record<SiteRole, () => string> = {
	administrator: m.role_administrator,
	auditor: m.role_auditor,
	security_officer: m.role_security_officer
};

/** What each role on a folder or device allows, in one line. */
export const ROLE_HINTS: Record<Role, () => string> = {
	list: m.access_role_hint_list,
	connect: m.access_role_hint_connect,
	reveal: m.access_role_hint_reveal,
	edit: m.access_role_hint_edit,
	manage: m.access_role_hint_manage
};
