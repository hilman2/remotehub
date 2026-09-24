<script lang="ts">
	/** remotehub's mark: an R in a rounded frame, drawn in the accent. */
	let { size = 26 }: { size?: number } = $props();
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 26 26"
	fill="none"
	aria-hidden="true"
	class="text-accent"
>
	<rect x="1" y="1" width="24" height="24" rx="7" stroke="currentColor" stroke-width="2" />
	<path
		d="M8 17V9h5.5a2.5 2.5 0 0 1 0 5H8m5 0 4 3"
		stroke="currentColor"
		stroke-width="2"
		stroke-linecap="round"
		stroke-linejoin="round"
	/>
</svg>
