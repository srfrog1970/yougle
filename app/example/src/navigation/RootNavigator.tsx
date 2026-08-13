import { createNativeStackNavigator } from '@react-navigation/native-stack';
import React from 'react';

import BackupExportScreen from '../screens/BackupExportScreen';
import ChatScreen from '../screens/ChatScreen';
import ConversationListScreen from '../screens/ConversationListScreen';
import ManageMailboxScreen from '../screens/ManageMailboxScreen';
import NodeSetupScreen from '../screens/NodeSetupScreen';
import PairingScreen from '../screens/PairingScreen';
import RecoveryPhraseScreen from '../screens/RecoveryPhraseScreen';
import ServerMailboxSetupScreen from '../screens/ServerMailboxSetupScreen';
import type { RootStackParamList } from './types';

const Stack = createNativeStackNavigator<RootStackParamList>();

export default function RootNavigator() {
  return (
    <Stack.Navigator initialRouteName="ConversationList">
      <Stack.Screen
        name="ConversationList"
        component={ConversationListScreen}
        options={{ headerShown: false }}
      />
      <Stack.Screen name="Chat" component={ChatScreen} />
      <Stack.Screen
        name="Pairing"
        component={PairingScreen}
        options={{ title: 'Pair a contact' }}
      />
      <Stack.Screen
        name="ManageMailbox"
        component={ManageMailboxScreen}
        options={{ title: 'Mailbox' }}
      />
      <Stack.Screen
        name="ServerMailboxSetup"
        component={ServerMailboxSetupScreen}
        options={{ title: 'Add Server mailbox' }}
      />
      <Stack.Screen
        name="NodeSetup"
        component={NodeSetupScreen}
        options={{ title: 'Set up your own node' }}
      />
      <Stack.Screen
        name="RecoveryPhrase"
        component={RecoveryPhraseScreen}
        options={{ title: 'Recovery phrase' }}
      />
      <Stack.Screen
        name="BackupExport"
        component={BackupExportScreen}
        options={{ title: 'Export backup' }}
      />
    </Stack.Navigator>
  );
}
