<script lang="ts">

  let password = $state('');
  let username = $state('');

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();

    let code;
    let token;
    try {
      const response = await fetch("/api/rooms/create", {
        method: "POST",
        headers: {
          'Accept': 'application/json',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({username, password: password.length > 0 ? password : undefined }),
      });

      ({ code, token } = await response.json());
    }
    catch (e) {
      console.log(e);
      return;
    }

    // TODO secure in prod
    document.cookie = `token=${token}; SameSite=strict`;
    window.location.href = `/room/${code}`;
  }
</script>

<form id="createForm" onsubmit={handleSubmit}>
  <label for="createFormUsernameInput">your name:</label>
  <input id="createFormUsernameInput" bind:value={username} required />
  <br>
  <label for="createFormPasswordInput">(optional) room password:</label>
  <input id="createFormPasswordInput" type="password" bind:value={password} />
  <br>
  <input type="submit" value="create" />
</form>
