<script lang="ts">
	import { page } from '$app/state';
</script>

{#if page.status = 404}
  <h1>not found</h1>
{:else}
  <h1>error</h1>
{/if}
