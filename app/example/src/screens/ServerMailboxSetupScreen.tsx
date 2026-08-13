// docs/PRD.md §7: "Server mailbox setup screen — enter connection details
// for a self-hosted mailbox the user is already running elsewhere."

import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import React, { useCallback, useState } from 'react';
import { Pressable, StyleSheet, Text, TextInput, View } from 'react-native';

import { describeError, useClient } from '../lib/client';
import type { RootStackParamList } from '../navigation/types';

export default function ServerMailboxSetupScreen() {
  const client = useClient();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();

  const [addr, setAddr] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onSave = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      await client.setOwnServerAddr(addr.trim());
      navigation.goBack();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  }, [client, addr, navigation]);

  return (
    <View style={styles.container}>
      <Text style={styles.instructions}>
        Paste the address your `pm-node` binary printed when it started.
      </Text>
      <TextInput
        style={styles.input}
        value={addr}
        onChangeText={setAddr}
        placeholder="Server address"
        multiline
        autoCapitalize="none"
        autoCorrect={false}
      />
      {error && <Text style={styles.error}>{error}</Text>}
      <Pressable
        style={[styles.primaryButton, (!addr.trim() || saving) && styles.buttonDisabled]}
        disabled={!addr.trim() || saving}
        onPress={onSave}>
        <Text style={styles.primaryButtonText}>
          {saving ? 'Saving…' : 'Save'}
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff', padding: 16 },
  instructions: { color: '#666', marginBottom: 16 },
  input: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    minHeight: 80,
    fontSize: 12,
    marginBottom: 16,
  },
  error: { color: 'crimson', marginBottom: 12 },
  primaryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    padding: 14,
    alignItems: 'center',
  },
  primaryButtonText: { color: '#fff', fontWeight: '700' },
  buttonDisabled: { opacity: 0.5 },
});
