// docs/PRD.md §7: "Chat view — text messages; status shown as sent /
// delivered / failed-to-deliver... No read receipts, no attachments."
//
// M5 scope note: only the Server-mailbox delivery path is wired up in
// pm-core yet (Local-to-local live delivery and a true "delivered" receipt
// are M6 — see pm-core::client's module doc comment). pm-core's `send`
// only persists a message once the write to the recipient's Server
// actually succeeds, so presence in `messagesForContact` already means
// "sent" — there's nothing further to track here honestly yet. A failed
// send throws, is shown inline, and leaves the compose box untouched so
// the user can retry, matching §7's local-failure behavior.

import { useFocusEffect, useNavigation, useRoute } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RouteProp } from '@react-navigation/native';
import React, { useCallback, useState } from 'react';
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import type { FfiMessage } from 'yougle-native';

import { describeError, useClient } from '../lib/client';
import { bufferToHex, bufferToText, textToBuffer } from '../lib/bytes';
import type { RootStackParamList } from '../navigation/types';

export default function ChatScreen() {
  const client = useClient();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  const route = useRoute<RouteProp<RootStackParamList, 'Chat'>>();
  const { contactId, displayName } = route.params;

  const [messages, setMessages] = useState<FfiMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  React.useEffect(() => {
    navigation.setOptions({ title: displayName ?? 'Conversation' });
  }, [navigation, displayName]);

  const load = useCallback(async () => {
    try {
      await client.sync();
    } catch {
      // No Server configured, or unreachable — local history still loads.
    }
    const msgs = await client.messagesForContact(contactId);
    setMessages(msgs);
  }, [client, contactId]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load])
  );

  const onSend = useCallback(async () => {
    const text = draft.trim();
    if (!text) return;
    setSending(true);
    setError(null);
    try {
      await client.send(contactId, textToBuffer(text));
      setDraft('');
      await load();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSending(false);
    }
  }, [client, contactId, draft, load]);

  return (
    <KeyboardAvoidingView
      style={styles.container}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <FlatList
        style={styles.list}
        data={messages}
        keyExtractor={m => bufferToHex(m.msgId)}
        renderItem={({ item }) => (
          <View
            style={[
              styles.bubble,
              item.outgoing ? styles.bubbleOut : styles.bubbleIn,
            ]}>
            <Text style={item.outgoing ? styles.bubbleTextOut : styles.bubbleTextIn}>
              {bufferToText(item.plaintext)}
            </Text>
          </View>
        )}
        ListEmptyComponent={
          <Text style={styles.empty}>No messages yet — say hello.</Text>
        }
      />

      {error && <Text style={styles.error}>{error}</Text>}

      <View style={styles.composeRow}>
        <TextInput
          style={styles.input}
          value={draft}
          onChangeText={setDraft}
          placeholder="Message"
          multiline
        />
        <Pressable
          style={[styles.sendButton, sending && styles.sendButtonDisabled]}
          disabled={sending}
          onPress={onSend}>
          <Text style={styles.sendButtonText}>Send</Text>
        </Pressable>
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff' },
  list: { flex: 1, padding: 12 },
  empty: { color: '#888', textAlign: 'center', marginTop: 32 },
  bubble: {
    maxWidth: '80%',
    padding: 10,
    borderRadius: 12,
    marginVertical: 4,
  },
  bubbleOut: { backgroundColor: '#007aff', alignSelf: 'flex-end' },
  bubbleIn: { backgroundColor: '#eee', alignSelf: 'flex-start' },
  bubbleTextOut: { color: '#fff' },
  bubbleTextIn: { color: '#000' },
  error: { color: 'crimson', paddingHorizontal: 12 },
  composeRow: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    padding: 12,
    gap: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: '#ddd',
  },
  input: {
    flex: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    maxHeight: 120,
  },
  sendButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    paddingHorizontal: 16,
    paddingVertical: 10,
  },
  sendButtonDisabled: { opacity: 0.5 },
  sendButtonText: { color: '#fff', fontWeight: '700' },
});
