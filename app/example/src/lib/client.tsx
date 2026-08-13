// The opened FfiClient, provided as React context. There's exactly one
// identity per device (per docs/PRD.md's constraints), so this owns the
// one-time "do we have a seed phrase yet" check at startup and exposes the
// three ways a client can come into being: silently created (first
// launch), restored from a Server, or restored from a manually-imported
// backup file (docs/PRD.md's Flow 5 / §7 Recovery screen).

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';
import RNFS from 'react-native-fs';
import * as Keychain from 'react-native-keychain';
import { FfiClient, generateSeedPhrase } from 'yougle-native';
import type { FfiClientLike } from 'yougle-native';

const STORE_PATH = `${RNFS.DocumentDirectoryPath}/yougle.sqlite`;
const KEYCHAIN_SERVICE = 'yougle-seed-phrase';

export async function loadSeedPhrase(): Promise<string | null> {
  const creds = await Keychain.getGenericPassword({ service: KEYCHAIN_SERVICE });
  return creds ? creds.password : null;
}

async function saveSeedPhrase(phrase: string): Promise<void> {
  await Keychain.setGenericPassword('yougle', phrase, { service: KEYCHAIN_SERVICE });
}

export function describeError(e: unknown): string {
  const asAny = e as any;
  if (asAny && Array.isArray(asAny.inner) && typeof asAny.inner[0] === 'string') {
    return asAny.inner[0];
  }
  if (e instanceof Error) return e.message;
  return String(e);
}

type ClientState =
  | { status: 'loading' }
  | { status: 'needs-identity' }
  | { status: 'ready'; client: FfiClientLike }
  | { status: 'error'; message: string };

type ClientContextValue = {
  state: ClientState;
  /** First launch: silently generate a new identity. */
  createIdentity: () => Promise<void>;
  /** Recovery: identity from phrase alone, or + contacts/history from a Server. */
  restoreFromPhrase: (phrase: string, serverAddr?: string) => Promise<void>;
  /** Recovery: identity from phrase, contacts/history from an imported backup file. */
  importFromBackup: (phrase: string, backupBytes: ArrayBuffer) => Promise<void>;
  refresh: () => void;
};

const ClientContext = createContext<ClientContextValue | null>(null);

export function ClientProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<ClientState>({ status: 'loading' });

  const openExisting = useCallback(async () => {
    setState({ status: 'loading' });
    try {
      const phrase = await loadSeedPhrase();
      if (!phrase) {
        setState({ status: 'needs-identity' });
        return;
      }
      const client = await FfiClient.open(phrase, STORE_PATH);
      setState({ status: 'ready', client });
    } catch (e) {
      setState({ status: 'error', message: describeError(e) });
    }
  }, []);

  useEffect(() => {
    openExisting();
  }, [openExisting]);

  const createIdentity = useCallback(async () => {
    setState({ status: 'loading' });
    try {
      const phrase = generateSeedPhrase();
      const client = await FfiClient.open(phrase, STORE_PATH);
      await saveSeedPhrase(phrase);
      setState({ status: 'ready', client });
    } catch (e) {
      setState({ status: 'error', message: describeError(e) });
    }
  }, []);

  const restoreFromPhrase = useCallback(async (phrase: string, serverAddr?: string) => {
    setState({ status: 'loading' });
    try {
      const client = serverAddr
        ? await FfiClient.restore(phrase, STORE_PATH, serverAddr)
        : await FfiClient.open(phrase, STORE_PATH);
      await saveSeedPhrase(phrase);
      setState({ status: 'ready', client });
    } catch (e) {
      setState({ status: 'error', message: describeError(e) });
    }
  }, []);

  const importFromBackup = useCallback(async (phrase: string, backupBytes: ArrayBuffer) => {
    setState({ status: 'loading' });
    try {
      const client = await FfiClient.importBackup(phrase, STORE_PATH, backupBytes);
      await saveSeedPhrase(phrase);
      setState({ status: 'ready', client });
    } catch (e) {
      setState({ status: 'error', message: describeError(e) });
    }
  }, []);

  const value = useMemo<ClientContextValue>(
    () => ({
      state,
      createIdentity,
      restoreFromPhrase,
      importFromBackup,
      refresh: openExisting,
    }),
    [state, createIdentity, restoreFromPhrase, importFromBackup, openExisting]
  );

  return <ClientContext.Provider value={value}>{children}</ClientContext.Provider>;
}

export function useClientState(): ClientContextValue {
  const ctx = useContext(ClientContext);
  if (!ctx) throw new Error('useClientState must be used within a ClientProvider');
  return ctx;
}

/** The opened client. Only valid to call from screens mounted while `state.status === 'ready'`. */
export function useClient(): FfiClientLike {
  const { state } = useClientState();
  if (state.status !== 'ready') {
    throw new Error('useClient() called before the client was ready');
  }
  return state.client;
}
