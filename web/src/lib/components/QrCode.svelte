<script lang="ts">
	/**
	 * A QR code of `value`, drawn as SVG in the page: nothing leaves the
	 * browser, which matters for secrets like a TOTP key.
	 */
	import { encode } from 'uqr';

	let { value, label }: { value: string; label: string } = $props();

	const qr = $derived(encode(value, { border: 2 }));
	// One square per dark module, all in a single path.
	const path = $derived(
		qr.data.flatMap((row, y) => row.map((dark, x) => (dark ? `M${x} ${y}h1v1h-1z` : ''))).join('')
	);
</script>

<svg
	viewBox="0 0 {qr.size} {qr.size}"
	class="size-44 rounded-lg bg-white"
	role="img"
	aria-label={label}
	shape-rendering="crispEdges"
>
	<path d={path} fill="black" />
</svg>
