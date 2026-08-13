// docs/PRD.md §7: "Pairing screen — dual-mode: display own QR code, scan a
// contact's QR code. No mailbox selection involved."
//
// M5 scope note: camera-based QR *scanning* isn't wired up in this pass
// (see app/README.md and the M5 plan — it can't be verified in a headless
// WSL2 emulator, and adding a native camera dependency untested is real
// risk for no verifiable benefit here). Pairing is instead built around an
// equally first-class *paste-a-code* path — the QR is generated from, and
// the pasted code decodes to, the exact same payload, so scanning can be
// added later as one more way to fill the same text field, not a
// different pairing mechanism.

import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import React, { useCallback, useEffect, useState } from 'react';
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import QRCode from 'react-native-qrcode-svg';
import type { FfiPairingPayload } from 'yougle-native';

import { describeError, useClient } from '../lib/client';
import { decodePairingPayload, encodePairingPayload } from '../lib/pairing';
import type { RootStackParamList } from '../navigation/types';

type Mode = 'my-code' | 'enter-code';

export default function PairingScreen() {
  const client = useClient();
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();

  const [mode, setMode] = useState<Mode>('my-code');
  const [myPayload, setMyPayload] = useState<FfiPairingPayload | null>(null);
  const [myCode, setMyCode] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const regenerate = useCallback(async () => {
    setError(null);
    try {
      const payload = await client.pairingPayload();
      setMyPayload(payload);
      setMyCode(encodePairingPayload(payload));
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    regenerate();
  }, [regenerate]);

  const [theirCode, setTheirCode] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [adding, setAdding] = useState(false);

  const onAddContact = useCallback(async () => {
    if (!myPayload) {
      setError('My own pairing code is not ready yet — try again in a moment.');
      return;
    }
    setError(null);
    setAdding(true);
    try {
      const their = decodePairingPayload(theirCode);
      await client.addContactFromPayload(
        their,
        myPayload.nonce,
        displayName.trim() || undefined
      );
      navigation.navigate('ConversationList');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setAdding(false);
    }
  }, [client, myPayload, theirCode, displayName, navigation]);

  return (
    <View style={styles.container}>
      <View style={styles.tabs}>
        <Pressable
          style={[styles.tab, mode === 'my-code' && styles.tabActive]}
          onPress={() => setMode('my-code')}>
          <Text style={mode === 'my-code' ? styles.tabTextActive : styles.tabText}>
            My code
          </Text>
        </Pressable>
        <Pressable
          style={[styles.tab, mode === 'enter-code' && styles.tabActive]}
          onPress={() => setMode('enter-code')}>
          <Text style={mode === 'enter-code' ? styles.tabTextActive : styles.tabText}>
            Enter code
          </Text>
        </Pressable>
      </View>

      {error && <Text style={styles.error}>{error}</Text>}

      {mode === 'my-code' ? (
        <ScrollView contentContainerStyle={styles.myCodeContainer}>
          <Text style={styles.instructions}>
            Show this to a contact in person, or share the code below.
          </Text>
          {myCode && (
            <>
              <View style={styles.qrWrap}>
                <QRCode value={myCode} size={220} />
              </View>
              <Text selectable style={styles.codeText}>
                {myCode}
              </Text>
            </>
          )}
          <Pressable style={styles.secondaryButton} onPress={regenerate}>
            <Text style={styles.secondaryButtonText}>Regenerate code</Text>
          </Pressable>
        </ScrollView>
      ) : (
        <ScrollView contentContainerStyle={styles.enterCodeContainer}>
          <Text style={styles.instructions}>
            Paste the code your contact shared with you.
          </Text>
          <TextInput
            style={styles.codeInput}
            value={theirCode}
            onChangeText={setTheirCode}
            placeholder="yougle-pair-v1:..."
            multiline
            autoCapitalize="none"
            autoCorrect={false}
            // M6 lengthened pairing codes enough (the added transportKey
            // field) to hit a real Android/Fabric TextInput crash
            // ("TextLayoutManager... Required value was null") on long
            // unbroken strings under the default 'highQuality' break
            // strategy — 'simple' is the documented workaround.
            textBreakStrategy="simple"
          />
          <TextInput
            style={styles.input}
            value={displayName}
            onChangeText={setDisplayName}
            placeholder="Name this contact (optional)"
          />
          <Pressable
            style={[styles.primaryButton, adding && styles.buttonDisabled]}
            disabled={adding || !theirCode.trim()}
            onPress={onAddContact}>
            <Text style={styles.primaryButtonText}>
              {adding ? 'Adding…' : 'Add contact'}
            </Text>
          </Pressable>
        </ScrollView>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff' },
  tabs: { flexDirection: 'row', padding: 12, gap: 8 },
  tab: {
    flex: 1,
    paddingVertical: 10,
    alignItems: 'center',
    backgroundColor: '#f0f0f0',
    borderRadius: 8,
  },
  tabActive: { backgroundColor: '#007aff' },
  tabText: { fontWeight: '600', color: '#333' },
  tabTextActive: { fontWeight: '600', color: '#fff' },
  error: { color: 'crimson', paddingHorizontal: 16 },
  instructions: { color: '#666', textAlign: 'center', marginBottom: 16 },
  myCodeContainer: { padding: 16, alignItems: 'center' },
  qrWrap: { padding: 16, backgroundColor: '#fff' },
  codeText: {
    fontSize: 12,
    color: '#444',
    marginTop: 16,
    textAlign: 'center',
  },
  enterCodeContainer: { padding: 16 },
  codeInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    minHeight: 80,
    fontSize: 12,
    marginBottom: 12,
  },
  input: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#ccc',
    borderRadius: 8,
    padding: 12,
    marginBottom: 16,
  },
  primaryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    padding: 14,
    alignItems: 'center',
  },
  primaryButtonText: { color: '#fff', fontWeight: '700' },
  secondaryButton: { marginTop: 20, padding: 10 },
  secondaryButtonText: { color: '#007aff', fontWeight: '600' },
  buttonDisabled: { opacity: 0.5 },
});
