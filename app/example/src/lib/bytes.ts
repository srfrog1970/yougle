// ArrayBuffer <-> string helpers. uniffi maps Rust's Vec<u8> to ArrayBuffer
// on the TS side (see app/src/generated/pm_ffi.ts) — these bridge that to
// what the UI actually needs: hex for compact display, UTF-8 for message
// text.

export function bufferToHex(buf: ArrayBuffer): string {
  return Array.from(new Uint8Array(buf))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

export function hexToBuffer(hex: string): ArrayBuffer {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes.buffer;
}

export function shortHex(buf: ArrayBuffer, chars = 8): string {
  return bufferToHex(buf).slice(0, chars);
}

export function textToBuffer(text: string): ArrayBuffer {
  return new TextEncoder().encode(text).buffer as ArrayBuffer;
}

export function bufferToText(buf: ArrayBuffer): string {
  return new TextDecoder().decode(buf);
}

// react-native-fs reads/writes file contents as base64 strings, not raw
// bytes — these bridge that to the ArrayBuffer the FFI layer expects
// (backup export/import). Chunked so a large backup (message history
// included) doesn't blow the call stack via a giant String.fromCharCode
// spread.
const CHUNK_SIZE = 0x8000;

export function bufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = '';
  for (let i = 0; i < bytes.length; i += CHUNK_SIZE) {
    const chunk = bytes.subarray(i, i + CHUNK_SIZE);
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

export function base64ToBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes.buffer;
}
