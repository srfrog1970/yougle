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

// Not `new TextDecoder().decode(buf)` — Hermes (RN's JS engine) ships
// `TextEncoder` but not `TextDecoder`, so that throws "Property
// 'TextDecoder' doesn't exist" the moment any message actually renders.
// Manual UTF-8 decode instead, encoding non-BMP code points as UTF-16
// surrogate pairs the way `String.fromCharCode` requires.
export function bufferToText(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let result = '';
  let i = 0;
  while (i < bytes.length) {
    const byte1 = bytes[i++];
    if (byte1 < 0x80) {
      result += String.fromCharCode(byte1);
    } else if (byte1 < 0xe0) {
      const byte2 = bytes[i++];
      result += String.fromCharCode(((byte1 & 0x1f) << 6) | (byte2 & 0x3f));
    } else if (byte1 < 0xf0) {
      const byte2 = bytes[i++];
      const byte3 = bytes[i++];
      result += String.fromCharCode(
        ((byte1 & 0x0f) << 12) | ((byte2 & 0x3f) << 6) | (byte3 & 0x3f)
      );
    } else {
      const byte2 = bytes[i++];
      const byte3 = bytes[i++];
      const byte4 = bytes[i++];
      const codepoint =
        ((byte1 & 0x07) << 18) |
        ((byte2 & 0x3f) << 12) |
        ((byte3 & 0x3f) << 6) |
        (byte4 & 0x3f);
      const cp = codepoint - 0x10000;
      result += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
    }
  }
  return result;
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
