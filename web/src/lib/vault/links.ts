/**
 * Links between the vault and the devices page (#190): which entry or
 * device is chosen lives in the query, so a link opens it.
 */
import { resolve } from '$app/paths';
import type { ResolvedPathname } from '$app/types';

/** The page at `path` with `name` set to `id`, typed as one of the app's addresses. */
const chosen = (path: '/' | '/vault', name: string, id: string) =>
	`${resolve(path)}?${new URLSearchParams({ [name]: id })}` as ResolvedPathname;

/** The vault with a shared credential chosen. */
export const vaultHref = (credentialId: string) => chosen('/vault', 'credential', credentialId);

/** The devices page with a device chosen. */
export const deviceHref = (deviceId: string) => chosen('/', 'device', deviceId);
