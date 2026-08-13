// Hermes (RN's JS engine) provides these globals at runtime, but
// @react-native/typescript-config's lib set doesn't declare them (it's not
// "dom", and they're not part of the TS core JS lib) — ambient
// declarations only, no implementation.

declare function btoa(data: string): string;
declare function atob(data: string): string;

declare class TextEncoder {
  encode(input?: string): Uint8Array;
}

declare class TextDecoder {
  constructor(label?: string, options?: { fatal?: boolean; ignoreBOM?: boolean });
  decode(input?: ArrayBuffer | ArrayBufferView): string;
}
