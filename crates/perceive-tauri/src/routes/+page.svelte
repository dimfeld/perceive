<script lang="ts">
  import { appContext } from '$lib/context';
  import { invoke } from '@tauri-apps/api/tauri';
  import { TextField } from 'svelte-ux';
  import debounce from 'just-debounce-it';

  const { loaded } = appContext();

  let query = '';
  let results = [];
  async function doSearch() {
    if (query) {
      results = await invoke('search', { query });
    }
  }

  const search = debounce(doSearch, 50);

  function itemLabels(item) {
    let main = item.metadata.name || item.external_id;
    let secondary = item.metadata.name ? item.external_id : '';
    let time = item.metadata.mtime || item.metadata.atime;

    return {
      main,
      secondary,
      time: '',
    };
  }
</script>

<div class="flex flex-col h-screen">
  <p class="text-red-500">{$loaded.status}</p>

  <TextField
    type="search"
    bind:value={query}
    debounceChange={50}
    on:change={search}
    label="Search"
    placeholder="Find something"
  />

  <p>Results</p>
  <ol class="flex-1 overflow-y-auto">
    <li />
    <div class="overflow-hidden bg-white shadow sm:rounded-md">
      <ul class="divide-y divide-gray-200">
        {#each results as result}
          {@const labels = itemLabels(result)}
          <li
            class="relative bg-white py-5 px-4 focus-within:ring-2 focus-within:ring-inset focus-within:ring-indigo-600 hover:bg-gray-50"
          >
            <div class="flex justify-between space-x-3">
              <div class="min-w-0 flex-1">
                <div class="block focus:outline-none">
                  <span class="absolute inset-0" aria-hidden="true" />
                  <p class="truncate text-sm font-medium text-gray-900">{labels.main}</p>
                  <p class="truncate text-sm text-gray-500">{labels.secondary}</p>
                </div>
              </div>
              <time class="flex-shrink-0 whitespace-nowrap text-sm text-gray-500"
                >{labels.time}</time
              >
            </div>
            <div class="mt-1">
              <p class="text-sm text-gray-600 line-clamp-2 max-h-10 overflow-hidden">
                {result.content}
              </p>
            </div>
          </li>
        {/each}
      </ul>
    </div>
  </ol>
</div>
