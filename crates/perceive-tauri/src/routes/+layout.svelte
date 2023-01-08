<script lang="ts">
  import { createAppContext } from '$lib/context';
  import '../app.postcss';

  createAppContext();
</script>

<slot />
