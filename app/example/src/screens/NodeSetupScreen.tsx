// Fills the gap between "Add Server mailbox" (which needs a running
// pm-node's address to paste in) and actually having one running: pm-node
// needs this device's own mailbox_key/server_transport_key as
// PM_NODE_MAILBOX_KEY/PM_NODE_TRANSPORT_KEY (see pm-node's own README/
// docs), derived from this identity's seed — nothing else in the app
// surfaces those values. Reveal-gated like RecoveryPhraseScreen: not as
// catastrophic as the seed phrase (these don't let anyone impersonate this
// identity or read message content), but someone who has both these
// values *and* reaches the resulting node can authenticate as its owner.

import React, { useCallback, useEffect, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { describeError, useClient } from '../lib/client';
import { bufferToHex } from '../lib/bytes';

export default function NodeSetupScreen() {
  const client = useClient();
  const [envText, setEnvText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);

  const load = useCallback(async () => {
    try {
      const [mailboxKey, serverTransportKey] = await Promise.all([
        client.mailboxKey(),
        client.serverTransportKey(),
      ]);
      setEnvText(
        `PM_NODE_MAILBOX_KEY=${bufferToHex(mailboxKey)}\n` +
          `PM_NODE_TRANSPORT_KEY=${bufferToHex(serverTransportKey)}`
      );
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <View style={styles.container}>
      <Text style={styles.instructions}>
        Running your own `pm-node` needs these two values, set as environment
        variables — see `deploy/README.md` for the full runbook. Once it's
        running, it prints an address: paste that into "Add Server mailbox",
        not these.
      </Text>
      <Text style={styles.warning}>
        Anyone who has both of these and can reach the resulting node could
        act as its owner. Keep them as private as the node's own
        configuration.
      </Text>

      {error && <Text style={styles.error}>{error}</Text>}

      {!revealed ? (
        <Pressable style={styles.revealButton} onPress={() => setRevealed(true)}>
          <Text style={styles.revealButtonText}>Tap to reveal</Text>
        </Pressable>
      ) : envText ? (
        <Text style={styles.envText} selectable>
          {envText}
        </Text>
      ) : (
        <Text style={styles.instructions}>Loading…</Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff', padding: 16 },
  instructions: { color: '#666', marginBottom: 16 },
  warning: { color: '#a05a00', marginBottom: 20 },
  error: { color: 'crimson', marginBottom: 12 },
  revealButton: {
    backgroundColor: '#f0f0f0',
    borderRadius: 8,
    padding: 20,
    alignItems: 'center',
  },
  revealButtonText: { fontWeight: '700' },
  envText: {
    fontFamily: 'monospace',
    backgroundColor: '#f7f7f7',
    borderRadius: 8,
    padding: 12,
    fontSize: 13,
  },
});
