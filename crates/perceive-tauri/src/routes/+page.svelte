<script lang="ts">
  import { appContext } from '$lib/context';

  const { loaded } = appContext();
</script>

Hello!
<p class="text-red-500">{$loaded.status}</p>

<input type="search" placeholder="Find something" />
