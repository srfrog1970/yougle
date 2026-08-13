// docs/PRD.md §7: "Conversation list — paired contacts and their most
// recent message." Also hosts the one-time backup nudge (§7's "Backup
// nudge": shown once, dismissible, never recurs).

import { useFocusEffect, useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import React, { useCallback, useState } from 'react';
import {
  FlatList,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import type { FfiContact, FfiMessage } from 'yougle-native';

import { useClient } from '../lib/client';
import { bufferToText, shortHex } from '../lib/bytes';
import type { RootStackParamList } from '../navigation/types';

const NUDGE_SHOWN_KEY = 'yougle-backup-nudge-shown';

type Row = { contact: FfiContact; lastMessage?: FfiMessage };

export default function ConversationListScreen() {
  const client = useClient();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();

  const [rows, setRows] = useState<Row[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showNudge, setShowNudge] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      try {
        await client.sync();
      } catch {
        // No Server configured, or it's unreachable — the conversation
        // list itself still works from local history either way.
      }
      const contacts = await client.listContacts();
      const withMessages = await Promise.all(
        contacts.map(async contact => {
          const messages = await client.messagesForContact(contact.id);
          return { contact, lastMessage: messages[messages.length - 1] };
        })
      );
      setRows(withMessages);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load])
  );

  React.useEffect(() => {
    AsyncStorage.getItem(NUDGE_SHOWN_KEY).then(value => {
      if (!value) setShowNudge(true);
    });
  }, []);

  const dismissNudge = useCallback(() => {
    setShowNudge(false);
    AsyncStorage.setItem(NUDGE_SHOWN_KEY, 'true');
  }, []);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    await load();
    setRefreshing(false);
  }, [load]);

  return (
    <View style={styles.container}>
      <View style={styles.header}>
        <Text style={styles.title}>Yougle</Text>
        <View style={styles.headerButtons}>
          <Pressable
            style={styles.headerButton}
            onPress={() => navigation.navigate('ManageMailbox')}>
            <Text style={styles.headerButtonText}>Mailbox</Text>
          </Pressable>
          <Pressable
            style={styles.headerButton}
            onPress={() => navigation.navigate('Pairing')}>
            <Text style={styles.headerButtonText}>+ Pair</Text>
          </Pressable>
        </View>
      </View>

      {showNudge && (
        <View style={styles.nudge}>
          <Text style={styles.nudgeText}>
            Back up your recovery phrase somewhere safe — it's the only way
            to recover your identity if you lose this device.
          </Text>
          <View style={styles.nudgeButtons}>
            <Pressable
              onPress={() => {
                dismissNudge();
                navigation.navigate('RecoveryPhrase');
              }}>
              <Text style={styles.nudgeAction}>View phrase</Text>
            </Pressable>
            <Pressable onPress={dismissNudge}>
              <Text style={styles.nudgeAction}>Dismiss</Text>
            </Pressable>
          </View>
        </View>
      )}

      {error && <Text style={styles.error}>{error}</Text>}

      <FlatList
        data={rows}
        keyExtractor={row => row.contact.id.toString()}
        refreshControl={
          <RefreshControl refreshing={refreshing} onRefresh={onRefresh} />
        }
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>
              No conversations yet. Tap "+ Pair" to add a contact.
            </Text>
          </View>
        }
        renderItem={({ item }) => (
          <Pressable
            style={styles.row}
            onPress={() =>
              navigation.navigate('Chat', {
                contactId: item.contact.id,
                displayName: item.contact.displayName,
              })
            }>
            <Text style={styles.rowName}>
              {item.contact.displayName ??
                shortHex(item.contact.identityKey)}
            </Text>
            <Text style={styles.rowPreview} numberOfLines={1}>
              {item.lastMessage
                ? bufferToText(item.lastMessage.plaintext)
                : 'No messages yet'}
            </Text>
          </Pressable>
        )}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff' },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: 16,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: '#ddd',
  },
  title: { fontSize: 22, fontWeight: '700' },
  headerButtons: { flexDirection: 'row', gap: 12 },
  headerButton: {
    paddingVertical: 6,
    paddingHorizontal: 10,
    backgroundColor: '#f0f0f0',
    borderRadius: 8,
  },
  headerButtonText: { fontWeight: '600' },
  nudge: {
    backgroundColor: '#fff8e1',
    padding: 12,
    margin: 12,
    borderRadius: 8,
  },
  nudgeText: { color: '#5c4a00' },
  nudgeButtons: {
    flexDirection: 'row',
    gap: 20,
    marginTop: 8,
  },
  nudgeAction: { fontWeight: '700', color: '#5c4a00' },
  error: { color: 'crimson', padding: 12 },
  empty: { padding: 32, alignItems: 'center' },
  emptyText: { color: '#888', textAlign: 'center' },
  row: {
    padding: 16,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: '#eee',
  },
  rowName: { fontSize: 16, fontWeight: '600' },
  rowPreview: { color: '#666', marginTop: 2 },
});
