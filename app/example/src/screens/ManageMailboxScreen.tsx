// docs/PRD.md §7: "Manage Mailbox screen — shows Local (always present,
// fixed) and, if configured, the user's single Server mailbox, with the
// ability to add, remove, or view its status."

import { useFocusEffect, useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import React, { useCallback, useState } from 'react';
import { Alert, Pressable, StyleSheet, Text, View } from 'react-native';

import { describeError, useClient } from '../lib/client';
import type { RootStackParamList } from '../navigation/types';

export default function ManageMailboxScreen() {
  const client = useClient();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();

  const [serverAddr, setServerAddr] = useState<string | null | undefined>(
    undefined
  );
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const addr = await client.ownServerAddr();
      setServerAddr(addr ?? null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load])
  );

  const onRemove = useCallback(() => {
    Alert.alert(
      'Remove Server mailbox?',
      "You'll fall back to Local-only. Messages already there stay on the server until you reconfigure it.",
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            try {
              await client.clearOwnServerAddr();
              setServerAddr(null);
            } catch (e) {
              setError(describeError(e));
            }
          },
        },
      ]
    );
  }, [client]);

  return (
    <View style={styles.container}>
      {error && <Text style={styles.error}>{error}</Text>}

      <View style={styles.card}>
        <Text style={styles.cardTitle}>Local</Text>
        <Text style={styles.cardBody}>
          Always present — this device, no setup required. Used automatically
          whenever no Server mailbox is configured.
        </Text>
      </View>

      <View style={styles.card}>
        <Text style={styles.cardTitle}>Server</Text>
        {serverAddr === undefined ? (
          <Text style={styles.cardBody}>Loading…</Text>
        ) : serverAddr ? (
          <>
            <Text style={styles.cardBody} selectable numberOfLines={2}>
              {serverAddr}
            </Text>
            <Pressable style={styles.dangerButton} onPress={onRemove}>
              <Text style={styles.dangerButtonText}>Remove</Text>
            </Pressable>
          </>
        ) : (
          <>
            <Text style={styles.cardBody}>
              Not configured. Add a self-hosted mailbox for durable,
              asynchronous delivery.
            </Text>
            <Pressable
              style={styles.primaryButton}
              onPress={() => navigation.navigate('ServerMailboxSetup')}>
              <Text style={styles.primaryButtonText}>Add Server mailbox</Text>
            </Pressable>
          </>
        )}
      </View>

      <Pressable
        style={styles.linkRow}
        onPress={() => navigation.navigate('RecoveryPhrase')}>
        <Text style={styles.linkText}>View recovery phrase</Text>
      </Pressable>
      <Pressable
        style={styles.linkRow}
        onPress={() => navigation.navigate('BackupExport')}>
        <Text style={styles.linkText}>Export backup</Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff', padding: 16 },
  error: { color: 'crimson', marginBottom: 12 },
  card: {
    backgroundColor: '#f7f7f7',
    borderRadius: 10,
    padding: 16,
    marginBottom: 16,
  },
  cardTitle: { fontSize: 16, fontWeight: '700', marginBottom: 6 },
  cardBody: { color: '#555' },
  primaryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    padding: 12,
    alignItems: 'center',
    marginTop: 12,
  },
  primaryButtonText: { color: '#fff', fontWeight: '700' },
  dangerButton: {
    borderRadius: 8,
    padding: 12,
    alignItems: 'center',
    marginTop: 12,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'crimson',
  },
  dangerButtonText: { color: 'crimson', fontWeight: '700' },
  linkRow: { paddingVertical: 14 },
  linkText: { color: '#007aff', fontWeight: '600' },
});
