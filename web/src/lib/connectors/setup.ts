/**
 * How to start a site connector (#171) and move it to a new release (#186),
 * with remotehub's address and release filled in, as docs/install.md
 * describes it. The token appears in none of the commands: on Linux it goes
 * into a file, on Windows the installer asks for it.
 *
 * remotehub serves the connector for Windows itself (#188), for servers that
 * reach remotehub but not GitHub. Where it does not, as in development, the
 * program comes from the GitHub release.
 */

const RELEASES = 'https://github.com/hilman2/remotehub/releases/download';

/** Where remotehub serves the connector for Windows and its hash. */
export const SERVED = {
	program: '/downloads/remotehub-connector.exe',
	sums: '/downloads/SHA256SUMS'
};

/**
 * Where the connector for Windows and its hashes are: on remotehub at
 * `origin` if it serves them, else in the GitHub release of `version`.
 */
export function windowsDownloads(
	version: string,
	served: string | null = null
): { program: string; sums: string } {
	if (served !== null) {
		return { program: `${served}${SERVED.program}`, sums: `${served}${SERVED.sums}` };
	}
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

/**
 * Lines that fetch the program and its hash from remotehub at `origin` into
 * a folder of their own, outside C:\Program Files, and stop if the hash does
 * not match. Without the progress bar, Windows PowerShell downloads many
 * times faster.
 */
function fetchFrom(origin: string): string[] {
	return [
		"$ProgressPreference = 'SilentlyContinue'",
		'Set-Location (New-Item -ItemType Directory -Force "$env:TEMP\\remotehub-connector")',
		`Invoke-WebRequest ${origin}${SERVED.program} -OutFile remotehub-connector.exe`,
		`Invoke-WebRequest ${origin}${SERVED.sums} -OutFile SHA256SUMS`,
		"$sum = (Get-Content .\\SHA256SUMS).Split(' ')[0]",
		"if ((Get-FileHash .\\remotehub-connector.exe -Algorithm SHA256).Hash -ne $sum) { throw 'remotehub-connector.exe does not match SHA256SUMS' }"
	];
}

/** Lines that show both hashes of files downloaded by hand, to compare. */
const SHOW_HASHES = [
	'(Get-FileHash .\\remotehub-connector.exe -Algorithm SHA256).Hash',
	'Select-String remotehub-connector.exe .\\SHA256SUMS'
];

/**
 * PowerShell lines that install the service; the installer asks for the
 * token. With `served`, remotehub's origin, they fetch the program first.
 */
export function windowsCommands(origin: string, served = false): string {
	return [
		...(served ? fetchFrom(origin) : SHOW_HASHES),
		`.\\remotehub-connector.exe install --url ${origin}`
	].join('\n');
}

/**
 * PowerShell lines that move the installed service to the release (#186);
 * token and settings stay. With `served`, remotehub's origin, they fetch the
 * program first.
 */
export function windowsUpdate(origin: string, served = false): string {
	return [...(served ? fetchFrom(origin) : SHOW_HASHES), '.\\remotehub-connector.exe update'].join(
		'\n'
	);
}
