<script lang="ts">
    import { onMount } from "svelte";
  import type { PageProps } from "./$types";

  let { data }: PageProps = $props();

  onMount(() => {
      const ws = new WebSocket(`/api/rooms/${data.code}/ws`);
      ws.onopen = () => {
        setInterval(() => {ws.send(JSON.stringify({Chat: {text: "hiiii"}})); console.log('sending');}, 2000);
      };
      ws.onmessage = (m) => {
        console.log(JSON.stringify(m.data));
      };
    })
</script>

<h1>welcome to {data.code}</h1>
