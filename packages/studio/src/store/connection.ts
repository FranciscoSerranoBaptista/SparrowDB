import { createStore } from "solid-js/store";

export type ConnectionStore = {
  baseUrl: string;
  apiKey: string;
  connected: boolean;
};

const LS_KEY = "sparrow_studio_connection";

function loadFromStorage(): ConnectionStore {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw) return JSON.parse(raw);
  } catch {
    // ignore
  }
  return { baseUrl: "", apiKey: "", connected: false };
}

export const [connection, setConnection] = createStore<ConnectionStore>(
  loadFromStorage()
);

export function saveConnection(updates: Partial<ConnectionStore>) {
  setConnection(updates);
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(connection));
  } catch {
    // ignore
  }
}
