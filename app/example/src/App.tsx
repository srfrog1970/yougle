import { NavigationContainer } from '@react-navigation/native';
import React from 'react';
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';

import { ClientProvider, useClientState } from './lib/client';
import RootNavigator from './navigation/RootNavigator';
import WelcomeScreen from './screens/WelcomeScreen';

function Gate() {
  const { state, refresh } = useClientState();

  switch (state.status) {
    case 'loading':
      return (
        <View style={styles.center}>
          <ActivityIndicator size="large" />
        </View>
      );
    case 'needs-identity':
      return <WelcomeScreen />;
    case 'error':
      return (
        <View style={styles.center}>
          <Text style={styles.errorTitle}>Something went wrong</Text>
          <Text style={styles.errorMessage}>{state.message}</Text>
          <Pressable style={styles.retryButton} onPress={refresh}>
            <Text style={styles.retryButtonText}>Retry</Text>
          </Pressable>
        </View>
      );
    case 'ready':
      return (
        <NavigationContainer>
          <RootNavigator />
        </NavigationContainer>
      );
  }
}

export default function App() {
  return (
    <SafeAreaProvider>
      <ClientProvider>
        <Gate />
      </ClientProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, justifyContent: 'center', alignItems: 'center', padding: 24 },
  errorTitle: { fontSize: 18, fontWeight: '700', marginBottom: 8 },
  errorMessage: { color: '#666', textAlign: 'center', marginBottom: 20 },
  retryButton: {
    backgroundColor: '#007aff',
    borderRadius: 8,
    paddingVertical: 12,
    paddingHorizontal: 24,
  },
  retryButtonText: { color: '#fff', fontWeight: '700' },
});
