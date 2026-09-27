/**
 * Which web pages a vault login belongs to (#201, ADR 0017). The server
 * checks the same rule before it hands a password out for filling
 * (crates/server/src/site.rs, which also explains it); the cases in
 * site-cases.json run against both.
 */
import { getDomain } from 'tldts';

/** An http or https URL; one without a scheme is taken as https. */
function parse(text: string): URL | null {
	const trimmed = text.trim();
	if (!trimmed) return null;
	try {
		const url = new URL(trimmed.includes('://') ? trimmed : `https://${trimmed}`);
		return url.protocol === 'http:' || url.protocol === 'https:' ? url : null;
	} catch {
		return null;
	}
}

/** The port a URL reaches, the scheme's default included. */
const port = (url: URL) => url.port || (url.protocol === 'https:' ? '443' : '80');

const bare = (host: string) => (host.endsWith('.') ? host.slice(0, -1) : host);

/** An IPv4 or IPv6 address, as `URL` writes it. */
const isAddress = (host: string) => host.startsWith('[') || /^\d+\.\d+\.\d+\.\d+$/.test(host);

/**
 * The registrable domain of `host`; null for a host without a dot, an
 * address, and a public suffix itself, which only match themselves. Private
 * suffixes such as github.io count, as in the server's list.
 */
function registrable(host: string): string | null {
	if (!host.includes('.') || isAddress(host)) return null;
	return getDomain(host, { allowPrivateDomains: true });
}

/**
 * Whether the page at `pageOrigin` (`location.origin`) belongs to the login
 * with the URL `loginUrl`.
 */
export function matches(loginUrl: string, pageOrigin: string): boolean {
	const login = parse(loginUrl);
	const page = parse(pageOrigin);
	if (!login || !page) return false;
	if (login.protocol !== page.protocol || port(login) !== port(page)) return false;
	const loginHost = bare(login.hostname);
	const pageHost = bare(page.hostname);
	if (loginHost === pageHost) return true;
	if (login.protocol !== 'https:' || isAddress(loginHost) || isAddress(pageHost)) return false;
	const a = registrable(loginHost);
	const b = registrable(pageHost);
	return a !== null && a === b;
}

/** The host of a page for the popup's header; null for anything not http(s). */
export function pageHost(url: string | undefined): string | null {
	const page = url ? parse(url) : null;
	return page && url?.includes('://') ? page.host : null;
}
