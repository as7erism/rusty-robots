<script lang="ts">
	import { page } from '$app/state';
   console.log("hiiii");
   console.log(JSON.stringify(page));
</script>

{#if page.status === 404}
  <h1>not found</h1>
{:else}
  <h1>error</h1>
{/if}
