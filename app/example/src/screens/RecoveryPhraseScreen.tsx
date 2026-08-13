// docs/PRD.md §7: "Recovery phrase screen (Settings) — view the recovery
// phrase on demand, at the user's own schedule." Not shown automatically —
// the user has to tap through, per the same section's non-blocking intent.

import React, { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { loadSeedPhrase } from '../lib/client';

export default function RecoveryPhraseScreen() {
  const [phrase, setPhrase] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);

  useEffect(() => {
    loadSeedPhrase().then(setPhrase);
  }, []);

  const words = phrase ? phrase.split(' ') : [];

  return (
    <View style={styles.container}>
      <Text style={styles.warning}>
        Anyone with these 24 words can restore your identity. Keep them
        private, written down somewhere safe — never in a screenshot or a
        note synced to the cloud.
      </Text>

      {!revealed ? (
        <Pressable style={styles.revealButton} onPress={() => setRevealed(true)}>
          <Text style={styles.revealButtonText}>Tap to reveal</Text>
        </Pressable>
      ) : (
        <View style={styles.grid}>
          {words.map((word, i) => (
            <View key={i} style={styles.wordCell}>
              <Text style={styles.wordIndex}>{i + 1}</Text>
              <Text style={styles.word} selectable>
                {word}
              </Text>
            </View>
          ))}
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#fff', padding: 16 },
  warning: { color: '#a05a00', marginBottom: 20 },
  revealButton: {
    backgroundColor: '#f0f0f0',
    borderRadius: 8,
    padding: 20,
    alignItems: 'center',
  },
  revealButtonText: { fontWeight: '700' },
  grid: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  wordCell: {
    flexDirection: 'row',
    width: '47%',
    padding: 8,
    backgroundColor: '#f7f7f7',
    borderRadius: 6,
    gap: 6,
  },
  wordIndex: { color: '#999', width: 20 },
  word: { fontWeight: '600' },
});
