import { getContext, setContext } from 'svelte';
import { writable, type Writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';
import index from 'just-index';

export type LoadStatus =
  | { status: 'loading' }
  | { status: 'loaded' }
  | { status: 'error'; error: string };

  export interface Source {
    id: number;
    name: string;
  }

export interface AppContext {
  loaded: Writable<LoadStatus>;
  sources: Writable<Record<number, Source>>;
}

export function appContext(): AppContext {
  return getContext('appContext');
}

export function createAppContext(): AppContext {
  const loaded = writable<LoadStatus>({ status: 'loading' });
  const sources = writable<Record<number, Source>>([]);

  // Both listen for the event, and do an explicit check, in case the load finished before the app started.
  listen<LoadStatus>('load_status', (event) => {
    loaded.set(event.payload);
  });

  invoke<LoadStatus>('load_status').then((result) => {
    loaded.set(result);
  });

  invoke<Source[]>('get_sources').then((result) => {
    sources.set(index(result, 'id'))
  });

  return setContext('appContext', {
    loaded,
    sources,
  });
}
