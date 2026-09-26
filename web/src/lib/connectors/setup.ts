/**
 * How to start a new site connector (#171), with remotehub's address and
 * release filled in, as docs/install.md describes it. The token appears in
 * none of the commands: on Linux it goes into a file, on Windows the
 * installer asks for it.
 */

const RELEASES = 'https://github.com/hilman2/remotehub/releases/download';

/** Where the connector for Windows and its hashes are for `version`. */
export function windowsDownloads(version: string): { program: string; sums: string } {
	const base = `${RELEASES}/v${version}`;
	return { program: `${base}/remotehub-connector.exe`, sums: `${base}/SHA256SUMS` };
}

/** Shell commands for a Linux host with Docker; the second waits for the token. */
export function dockerCommands(origin: string, version: string): string {
	return [
		'sudo install -d -m 700 /opt/remotehub-connector',
		"sudo sh -c 'cat > /opt/remotehub-connector/token'",
		'sudo chown 65532 /opt/remotehub-connector/token && sudo chmod 400 /opt/remotehub-connector/token',
		'sudo docker run -d --name remotehub-connector --restart unless-stopped --read-only \\',
		'  --cap-drop ALL --security-opt no-new-privileges \\',
		'  -v /opt/remotehub-connector/token:/run/secrets/token:ro \\',
		'  -v remotehub-connector-data:/var/lib/remotehub-connector \\',
		'  -p 127.0.0.1:8480:8480 \\',
		`  -e REMOTEHUB_URL=${origin} \\`,
		'  -e REMOTEHUB_CONNECTOR_TOKEN_FILE=/run/secrets/token \\',
		`  ghcr.io/hilman2/remotehub-connector:${version}`
	].join('\n');
}

/** PowerShell lines: compare the hash, then install; the installer asks for the token. */
export function windowsCommands(origin: string): string {
	return [
		'(Get-FileHash .\\remotehub-connector.exe -Algorithm SHA256).Hash',
		'Select-String remotehub-connector.exe .\\SHA256SUMS',
		`.\\remotehub-connector.exe install --url ${origin}`
	].join('\n');
}
