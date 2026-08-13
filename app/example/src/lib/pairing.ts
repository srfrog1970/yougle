// Packs/unpacks an FfiPairingPayload as one opaque code string — what
// actually gets shown as a QR code and as pasteable text on the Pairing
// screen. pm-ffi deliberately leaves this packing to the app layer (see
// FfiPairingPayload's doc comment in crates/pm-ffi/src/lib.rs) since it's
// UI/UX surface, not protocol.

import type { FfiPairingPayload } from 'yougle-native';
import { bufferToHex, hexToBuffer } from './bytes';

const PREFIX = 'yougle-pair-v1:';

type WirePayload = {
  identityKey: string;
  curve25519Key: string;
  transportKey: string;
  oneTimeKey: string;
  nonce: string;
  serverAddr?: string;
};

export function encodePairingPayload(payload: FfiPairingPayload): string {
  const wire: WirePayload = {
    identityKey: bufferToHex(payload.identityKey),
    curve25519Key: bufferToHex(payload.curve25519Key),
    transportKey: bufferToHex(payload.transportKey),
    oneTimeKey: bufferToHex(payload.oneTimeKey),
    nonce: bufferToHex(payload.nonce),
    serverAddr: payload.serverAddr ? bufferToHex(payload.serverAddr) : undefined,
  };
  return PREFIX + btoa(JSON.stringify(wire));
}

export function decodePairingPayload(code: string): FfiPairingPayload {
  const trimmed = code.trim();
  if (!trimmed.startsWith(PREFIX)) {
    throw new Error('Not a Yougle pairing code');
  }
  let wire: WirePayload;
  try {
    wire = JSON.parse(atob(trimmed.slice(PREFIX.length)));
  } catch {
    throw new Error('Pairing code is corrupt or incomplete');
  }
  return {
    identityKey: hexToBuffer(wire.identityKey),
    curve25519Key: hexToBuffer(wire.curve25519Key),
    transportKey: hexToBuffer(wire.transportKey),
    oneTimeKey: hexToBuffer(wire.oneTimeKey),
    nonce: hexToBuffer(wire.nonce),
    serverAddr: wire.serverAddr ? hexToBuffer(wire.serverAddr) : undefined,
  };
}
