/**
 * TRI-SYNC TypeScript Reference Client
 *
 * Implements the cross-language determinism primitives described in
 * docs/cross-language-determinism.md.  Any conforming implementation starting
 * from the same input MUST produce byte-for-byte identical output.
 *
 * Conformance test vectors (from §3.5 and §3.4 of the spec):
 *   verifyRootDigest() with the §3.5 state → "768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3"
 *   sha256Hex(new Uint8Array([0,0,0,0]))   → "df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119"
 */

// ---------------------------------------------------------------------------
// Canonical JSON (SPEC §1, RFC 8785)
// ---------------------------------------------------------------------------

type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };

/**
 * Serialize a JSON value to canonical form.
 *
 * Rules:
 *  - Object keys sorted by raw UTF-8 byte order (equivalent to `localeCompare`
 *    with no locale; for ASCII keys this is identical to `<` comparison).
 *  - No whitespace outside string values.
 *  - Control characters U+0000–U+001F escaped as \uXXXX with lowercase hex.
 *  - No Unicode normalization; raw UTF-8 bytes are preserved.
 *
 * @param value  Any JSON-serializable value.
 * @returns      Canonical JSON string.
 */
export function canonicalJson(value: JsonValue): string {
  if (value === null) return 'null';
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'number') return canonicalNumber(value);
  if (typeof value === 'string') return jsonString(value);
  if (Array.isArray(value)) {
    return '[' + value.map(canonicalJson).join(',') + ']';
  }
  // Object — sort keys by raw UTF-8 byte order.
  const entries = Object.entries(value as { [key: string]: JsonValue });
  entries.sort(([a], [b]) => {
    const ab = new TextEncoder().encode(a);
    const bb = new TextEncoder().encode(b);
    const len = Math.min(ab.length, bb.length);
    for (let i = 0; i < len; i++) {
      if (ab[i] !== bb[i]) return ab[i] - bb[i];
    }
    return ab.length - bb.length;
  });
  return '{' + entries.map(([k, v]) => jsonString(k) + ':' + canonicalJson(v)).join(',') + '}';
}

/**
 * Encode a number as canonical JSON.
 *
 * Integers are emitted without a decimal point. Floats use
 * `toPrecision` / `toFixed` minimally.  NaN and Infinity are not
 * representable per the spec and throw.
 */
function canonicalNumber(n: number): string {
  if (!isFinite(n)) throw new Error('INVALID_NUMERIC: ' + n);
  // Use JSON.stringify which produces correct canonical output for all finite numbers.
  return JSON.stringify(n);
}

/**
 * Encode a string as a JSON string with the canonical escape rules.
 *
 * Control characters U+0000–U+001F are escaped as \uXXXX with lowercase hex
 * (RFC 8785 §3.2.2).  Other characters are emitted as-is (UTF-8).
 */
function jsonString(s: string): string {
  let out = '"';
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    if (ch === '"') { out += '\\"'; continue; }
    if (ch === '\\') { out += '\\\\'; continue; }
    if (cp === 0x08) { out += '\\b'; continue; }
    if (cp === 0x0c) { out += '\\f'; continue; }
    if (cp === 0x0a) { out += '\\n'; continue; }
    if (cp === 0x0d) { out += '\\r'; continue; }
    if (cp === 0x09) { out += '\\t'; continue; }
    if (cp <= 0x1f) {
      out += '\\u' + cp.toString(16).padStart(4, '0');
      continue;
    }
    out += ch;
  }
  out += '"';
  return out;
}

// ---------------------------------------------------------------------------
// SHA-256 (SPEC §4)
// ---------------------------------------------------------------------------

/**
 * Compute SHA-256 of arbitrary bytes.
 *
 * Uses the Web Crypto API (available in browsers, Node 18+, Cloudflare Workers,
 * Deno, Bun, and any other modern JS runtime).
 *
 * @param data  Input bytes.
 * @returns     Lowercase 64-character hex digest string.
 */
export async function sha256Hex(data: Uint8Array): Promise<string> {
  const input = new Uint8Array(data.byteLength);
  input.set(data);
  const buf = await crypto.subtle.digest('SHA-256', input);
  return Array.from(new Uint8Array(buf))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

// ---------------------------------------------------------------------------
// Binary State Map (BSM) encoding (SPEC §3)
// ---------------------------------------------------------------------------

/** Supported BSM value types. */
export type BsmValue =
  | { type: 'Boolean'; value: boolean }
  | { type: 'Integer'; value: bigint }
  | { type: 'Decimal'; value: string }
  | { type: 'String'; value: string }
  | { type: 'Bytes'; value: Uint8Array }
  | { type: 'Null' };

const TYPE_BOOLEAN = 0x01;
const TYPE_INTEGER = 0x02;
const TYPE_DECIMAL = 0x03;
const TYPE_STRING  = 0x04;
const TYPE_BYTES   = 0x05;
const TYPE_NULL    = 0x06;

/**
 * Encode a Binary State Map to bytes.
 *
 * The map is sorted by key in raw UTF-8 byte order before encoding.
 * This is the exact byte sequence that SHA-256 is computed over for
 * the root digest.
 *
 * @param entries  Map of key → BsmValue pairs.
 * @returns        BSM wire bytes.
 */
export function encodeBsm(entries: Map<string, BsmValue>): Uint8Array {
  const encoder = new TextEncoder();

  // Sort keys by raw UTF-8 byte sequence.
  const sorted = [...entries.entries()].sort(([a], [b]) => {
    const ab = encoder.encode(a);
    const bb = encoder.encode(b);
    const len = Math.min(ab.length, bb.length);
    for (let i = 0; i < len; i++) {
      if (ab[i] !== bb[i]) return ab[i] - bb[i];
    }
    return ab.length - bb.length;
  });

  const parts: Uint8Array[] = [];

  // entry_count: u32 BE
  parts.push(u32be(sorted.length));

  for (const [key, val] of sorted) {
    const keyBytes = encoder.encode(key);
    // key_len: u16 BE
    parts.push(u16be(keyBytes.length));
    parts.push(keyBytes);
    parts.push(encodeValue(val, encoder));
  }

  return concat(parts);
}

function encodeValue(val: BsmValue, encoder: TextEncoder): Uint8Array {
  switch (val.type) {
    case 'Boolean':
      return new Uint8Array([TYPE_BOOLEAN, val.value ? 1 : 0]);

    case 'Integer': {
      const out = new Uint8Array(9);
      out[0] = TYPE_INTEGER;
      const view = new DataView(out.buffer);
      view.setBigInt64(1, val.value, false /* big-endian */);
      return out;
    }

    case 'Decimal': {
      const bytes = encoder.encode(val.value);
      return concat([new Uint8Array([TYPE_DECIMAL]), u32be(bytes.length), bytes]);
    }

    case 'String': {
      const bytes = encoder.encode(val.value);
      return concat([new Uint8Array([TYPE_STRING]), u32be(bytes.length), bytes]);
    }

    case 'Bytes':
      return concat([new Uint8Array([TYPE_BYTES]), u32be(val.value.length), val.value]);

    case 'Null':
      return new Uint8Array([TYPE_NULL]);
  }
}

// ---------------------------------------------------------------------------
// Root digest
// ---------------------------------------------------------------------------

/**
 * Compute the BSM root digest for a state map.
 *
 * Encodes the map to BSM wire format and returns SHA-256 as a lowercase hex string.
 *
 * Conformance check:
 *   The §3.5 test vector state produces:
 *   "768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3"
 *
 * @param entries  State map.
 * @returns        Lowercase 64-character hex root digest.
 */
export async function computeRootDigest(entries: Map<string, BsmValue>): Promise<string> {
  return sha256Hex(encodeBsm(entries));
}

/**
 * Verify that a state map produces the expected root digest.
 *
 * @param entries         State map.
 * @param expectedDigest  Expected 64-character lowercase hex digest.
 * @returns               `true` if the computed digest matches.
 */
export async function verifyRootDigest(
  entries: Map<string, BsmValue>,
  expectedDigest: string
): Promise<boolean> {
  const actual = await computeRootDigest(entries);
  return actual === expectedDigest.toLowerCase();
}

// ---------------------------------------------------------------------------
// Low-level binary helpers
// ---------------------------------------------------------------------------

function u32be(n: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setUint32(0, n, false);
  return buf;
}

function u16be(n: number): Uint8Array {
  const buf = new Uint8Array(2);
  new DataView(buf.buffer).setUint16(0, n, false);
  return buf;
}

function concat(arrays: Uint8Array[]): Uint8Array {
  const total = arrays.reduce((sum, a) => sum + a.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const a of arrays) {
    out.set(a, offset);
    offset += a.length;
  }
  return out;
}
