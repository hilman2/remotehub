/**
 * How to start a site connector (#171) and move it to a new release (#186),
 * with remotehub's address and release filled in, as docs/install.md
 * describes it. The token appears in
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
		dockerRun(origin, version)
	].join('\n');
}

/**
 * Shell commands that move a running connector to `version` (#186): the
 * container is replaced, the token file and the data volume stay.
 */
export function dockerUpdate(origin: string, version: string): string {
	return [
		`sudo docker pull ghcr.io/hilman2/remotehub-connector:${version}`,
		'sudo docker rm -f remotehub-connector',
		dockerRun(origin, version)
	].join('\n');
}

function dockerRun(origin: string, version: string): string {
	return [
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

const HASHES = [
	'(Get-FileHash .\\remotehub-connector.exe -Algorithm SHA256).Hash',
	'Select-String remotehub-connector.exe .\\SHA256SUMS'
];

/** PowerShell lines: compare the hash, then install; the installer asks for the token. */
export function windowsCommands(origin: string): string {
	return [...HASHES, `.\\remotehub-connector.exe install --url ${origin}`].join('\n');
}

/**
 * PowerShell lines that move the installed service to the downloaded
 * release (#186); token and settings stay.
 */
export function windowsUpdate(): string {
	return [...HASHES, '.\\remotehub-connector.exe update'].join('\n');
}
