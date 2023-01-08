import { getContext, setContext } from 'svelte';
import { writable, type Writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';

export type LoadStatus =
  | { status: 'loading' }
  | { status: 'loaded' }
  | { status: 'error'; error: string };

export interface AppContext {
  loaded: Writable<LoadStatus>;
}

export function appContext(): AppContext {
  return getContext('appContext');
}

export function createAppContext(): AppContext {
  const loaded = writable<LoadStatus>({ status: 'loading' });

  // Both listen for the event, and do an explicit check, in case the load finished before the app started.
  listen<LoadStatus>('load_status', (event) => {
    loaded.set(event.payload);
  });

  invoke<LoadStatus>('load_status').then((result) => {
    loaded.set(result);
  });

  return setContext('appContext', {
    loaded,
  });
}
