/**
 * TRI-SYNC TypeScript client — conformance tests.
 *
 * These tests mirror the conformance checklist in docs/cross-language-determinism.md.
 * Run with: npm test
 */

import { canonicalJson, sha256Hex, encodeBsm, computeRootDigest, verifyRootDigest } from './index.js';
import type { BsmValue } from './index.js';

let passed = 0;
let failed = 0;

function assert(desc: string, actual: unknown, expected: unknown): void {
  if (actual === expected) {
    console.log(`  PASS  ${desc}`);
    passed++;
  } else {
    console.error(`  FAIL  ${desc}`);
    console.error(`        expected: ${JSON.stringify(expected)}`);
    console.error(`        actual:   ${JSON.stringify(actual)}`);
    failed++;
  }
}

async function assertAsync(desc: string, actual: Promise<unknown> | unknown, expected: unknown): Promise<void> {
  const result = await actual;
  assert(desc, result, expected);
}

// ---------------------------------------------------------------------------
// 1. Canonical JSON
// ---------------------------------------------------------------------------

console.log('\n--- Canonical JSON ---');

assert('null', canonicalJson(null), 'null');
assert('true', canonicalJson(true), 'true');
assert('false', canonicalJson(false), 'false');
assert('integer 0', canonicalJson(0), '0');
assert('integer 42', canonicalJson(42), '42');
assert('negative -1', canonicalJson(-1), '-1');
assert('empty object', canonicalJson({}), '{}');
assert('empty array', canonicalJson([]), '[]');
assert('sorted keys', canonicalJson({ z: 1, a: 2, m: 3 }), '{"a":2,"m":3,"z":1}');
assert('array order preserved', canonicalJson([3, 1, 2]), '[3,1,2]');
assert('nested sorted keys', canonicalJson({ z: { b: 2, a: 1 }, a: [{ k: 2, j: 1 }] }),
  '{"a":[{"j":1,"k":2}],"z":{"a":1,"b":2}}');

// String escapes
assert('backslash and quote', canonicalJson('a"b\\c'), '"a\\"b\\\\c"');
assert('null byte', canonicalJson('\u0000'), '"\\u0000"');
assert('unit separator U+001F', canonicalJson('\u001f'), '"\\u001f"');
assert('newline', canonicalJson('\n'), '"\\n"');
assert('tab', canonicalJson('\t'), '"\\t"');
assert('backspace', canonicalJson('\b'), '"\\b"');
assert('form feed', canonicalJson('\f'), '"\\f"');
assert('carriage return', canonicalJson('\r'), '"\\r"');

// No Unicode normalization: NFC "café" and NFD "cafe\u0301" are distinct.
assert('NFC café', canonicalJson('\u0063\u0061\u0066\u00e9'), '"caf\u00e9"');
assert('NFD cafe+combining', canonicalJson('\u0063\u0061\u0066\u0065\u0301'), '"cafe\u0301"');

// ---------------------------------------------------------------------------
// 2. SHA-256
// ---------------------------------------------------------------------------

console.log('\n--- SHA-256 ---');

await assertAsync('empty BSM bytes → empty BSM digest',
  sha256Hex(new Uint8Array([0x00, 0x00, 0x00, 0x00])),
  'df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119');

await assertAsync('sha256("abc")',
  sha256Hex(new TextEncoder().encode('abc')),
  'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');

// ---------------------------------------------------------------------------
// 3. BSM encoding + root digest (§3.5 conformance vector)
// ---------------------------------------------------------------------------

console.log('\n--- BSM root digest (§3.5 conformance vector) ---');

const state = new Map<string, BsmValue>([
  ['tenant-a:counter', { type: 'Integer', value: 42n }],
  ['tenant-a:flag',    { type: 'Boolean', value: true }],
  ['tenant-a:ratio',   { type: 'Decimal', value: '3.14' }],
]);

await assertAsync('§3.5 root digest',
  computeRootDigest(state),
  '768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3');

await assertAsync('verifyRootDigest returns true for correct digest',
  verifyRootDigest(state, '768e154f65fb12f4419452ac76223006bf9097187294b0d9cec1260e22c664d3'),
  true);

await assertAsync('verifyRootDigest returns false for wrong digest',
  verifyRootDigest(state, '0000000000000000000000000000000000000000000000000000000000000000'),
  false);

// Empty BSM: 4 zero bytes → known digest
const emptyState = new Map<string, BsmValue>();
const emptyBsm = encodeBsm(emptyState);
assert('empty BSM is 4 bytes', emptyBsm.length, 4);
assert('empty BSM bytes', Array.from(emptyBsm).join(','), '0,0,0,0');

await assertAsync('empty BSM root digest',
  computeRootDigest(emptyState),
  'df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119');

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

console.log(`\n${passed + failed} tests: ${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
