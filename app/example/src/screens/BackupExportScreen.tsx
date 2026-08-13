// docs/PRD.md §7: "Backup export screen (Settings) — manually export an
// encrypted contacts-and-message-history file; the user manages where it's
// stored." The OS share sheet is how that hand-off happens — this screen
// never picks a storage location itself.

import React, { useCallback, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import RNFS from 'react-native-fs';
import Share from 'react-native-share';

import { describeError, useClient } from '../lib/client';
import { bufferToBase64 } from '../lib/bytes';

export default function BackupExportScreen() {
  const client = useClient();
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const onExport = useCallback(async () => {
    setExporting(true);
    setError(null);
    setDone(false);
    try {
      const backupBytes = await client.exportBackup();
      const filePath = `${RNFS.CachesDirectoryPath}/yougle-backup-${Date.now()}.yougle`;
      await RNFS.writeFile(filePath, bufferToBase64(backupBytes), 'base64');
      await Share.open({
        url: `file://${filePath}`,
        type: 'application/octet-stream',
        filename: 'yougle-backup.yougle',
      });
      setDone(true);
    } catch (e: any) {
      // The user cancelling the share sheet also lands here — not a real
      // error, so don't show it as one.
      if (e?.message !== 'User did not share') {
        setError(describeError(e));
      }
    } finally {
      setExporting(false);
    }
  }, [client]);

  return (
    <View style={styles.container}>
      <Text style={styles.instructions}>
        Exports an encrypted file containing your contacts and full message
        history. Anyone who has both this file and your recovery phrase can
        restore your account — store it somewhere only you control.
      </Text>

      {error && <Text style={styles.error}>{error}</Text>}
      {done && <Text style={styles.success}>Backup exported.</Text>}

      <Pressable
        style={[styles.primaryButton, exporting && styles.buttonDisabled]}
        disabled={exporting}
        onPress={onExport}>
        <Text style={styles.primaryButtonText}>
          {exporting ? 'Exporting…' : 'Export backup'}
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff', padding: 16 },
  instructions: { color: '#666', marginBottom: 20 },
  error: { color: 'crimson', marginBottom: 12 },
  success: { color: 'green', marginBottom: 12 },
  primaryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    padding: 14,
    alignItems: 'center',
  },
  primaryButtonText: { color: '#fff', fontWeight: '700' },
  buttonDisabled: { opacity: 0.5 },
});
