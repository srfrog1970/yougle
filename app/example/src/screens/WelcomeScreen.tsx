// docs/PRD.md Flow 1 ("First launch... nothing is explained about delivery
// mechanics up front") + Flow 5 ("Recovery on a new device"). No client
// exists yet at this point, so this isn't part of the main navigator — see
// App.tsx, which renders this in place of the stack until identity exists.

import React, { useCallback, useState } from 'react';
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import RNFS from 'react-native-fs';
import { pick } from '@react-native-documents/picker';

import { describeError, useClientState } from '../lib/client';
import { base64ToBuffer } from '../lib/bytes';

type Mode = 'menu' | 'recovery';

export default function WelcomeScreen() {
  const { createIdentity, restoreFromPhrase, importFromBackup } = useClientState();
  const [mode, setMode] = useState<Mode>('menu');
  const [phrase, setPhrase] = useState('');
  const [serverAddr, setServerAddr] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onImportFile = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      const [result] = await pick();
      const base64 = await RNFS.readFile(result.uri, 'base64');
      await importFromBackup(phrase.trim(), base64ToBuffer(base64));
    } catch (e: any) {
      if (e?.code !== 'DOCUMENT_PICKER_CANCELED') {
        setError(describeError(e));
      }
    } finally {
      setBusy(false);
    }
  }, [phrase, importFromBackup]);

  const onRestoreFromServer = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      await restoreFromPhrase(phrase.trim(), serverAddr.trim());
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  }, [phrase, serverAddr, restoreFromPhrase]);

  const onSkip = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      await restoreFromPhrase(phrase.trim());
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  }, [phrase, restoreFromPhrase]);

  if (mode === 'menu') {
    return (
      <View style={styles.center}>
        <Text style={styles.title}>Yougle</Text>
        {busy ? (
          <ActivityIndicator />
        ) : (
          <>
            <Pressable style={styles.primaryButton} onPress={createIdentity}>
              <Text style={styles.primaryButtonText}>Get started</Text>
            </Pressable>
            <Pressable style={styles.linkRow} onPress={() => setMode('recovery')}>
              <Text style={styles.linkText}>I have a recovery phrase</Text>
            </Pressable>
          </>
        )}
        {error && <Text style={styles.error}>{error}</Text>}
      </View>
    );
  }

  const wordCount = phrase.trim().split(/\s+/).filter(Boolean).length;

  return (
    <ScrollView contentContainerStyle={styles.recoveryContainer}>
      <Text style={styles.title}>Restore your identity</Text>
      <TextInput
        style={styles.phraseInput}
        value={phrase}
        onChangeText={setPhrase}
        placeholder="24-word recovery phrase"
        multiline
        autoCapitalize="none"
        autoCorrect={false}
      />
      <Text style={styles.wordCount}>{wordCount} / 24 words</Text>

      {error && <Text style={styles.error}>{error}</Text>}

      {busy ? (
        <ActivityIndicator style={styles.spinner} />
      ) : (
        <View style={styles.recoveryOptions}>
          <Text style={styles.sectionLabel}>
            Restore contacts and message history from:
          </Text>
          <TextInput
            style={styles.input}
            value={serverAddr}
            onChangeText={setServerAddr}
            placeholder="Your Server mailbox address"
            autoCapitalize="none"
            autoCorrect={false}
          />
          <Pressable
            style={[styles.primaryButton, wordCount !== 24 || !serverAddr.trim() ? styles.buttonDisabled : undefined]}
            disabled={wordCount !== 24 || !serverAddr.trim()}
            onPress={onRestoreFromServer}>
            <Text style={styles.primaryButtonText}>Restore from Server</Text>
          </Pressable>

          <Pressable
            style={[styles.secondaryButton, wordCount !== 24 && styles.buttonDisabled]}
            disabled={wordCount !== 24}
            onPress={onImportFile}>
            <Text style={styles.secondaryButtonText}>Import a backup file</Text>
          </Pressable>

          <Pressable
            style={[styles.linkRow, wordCount !== 24 && styles.buttonDisabled]}
            disabled={wordCount !== 24}
            onPress={onSkip}>
            <Text style={styles.linkText}>
              Skip — just restore my identity (no contacts/history)
            </Text>
          </Pressable>
        </View>
      )}

      <Pressable style={styles.linkRow} onPress={() => setMode('menu')}>
        <Text style={styles.linkText}>Back</Text>
      </Pressable>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, justifyContent: 'center', alignItems: 'center', padding: 24 },
  recoveryContainer: { flexGrow: 1, padding: 24, paddingTop: 60 },
  title: { fontSize: 28, fontWeight: '700', marginBottom: 24 },
  primaryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    paddingVertical: 14,
    paddingHorizontal: 32,
    alignItems: 'center',
  },
  primaryButtonText: { color: '#fff', fontWeight: '700', fontSize: 16 },
  secondaryButton: {
    borderRadius: 8,
    padding: 14,
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#007aff',
    marginTop: 12,
  },
  secondaryButtonText: { color: '#007aff', fontWeight: '700' },
  buttonDisabled: { opacity: 0.5 },
  linkRow: { marginTop: 16, alignItems: 'center' },
  linkText: { color: '#007aff', fontWeight: '600' },
  error: { color: 'crimson', marginTop: 16, textAlign: 'center' },
  phraseInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    minHeight: 80,
    marginBottom: 4,
  },
  wordCount: { color: '#888', marginBottom: 20 },
  sectionLabel: { color: '#666', marginBottom: 8 },
  input: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    marginBottom: 12,
  },
  recoveryOptions: { marginTop: 8 },
  spinner: { marginTop: 24 },
});
