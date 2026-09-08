import assert from "node:assert";
import test from "node:test";
import worker from "../src/index.js";

function createMockEnv() {
  const issued = new Map();
  const marketing = new Map();

  return {
    ISSUED_KEYS: {
      get: async (key) => issued.get(key) || null,
      put: async (key, value) => { issued.set(key, value); }
    },
    MARKETING_CONTACTS: {
      get: async (key) => marketing.get(key) || null,
      put: async (key, value) => { marketing.set(key, value); }
    }
  };
}

test("health endpoint", async () => {
  const req = new Request("https://api.trisync.dev/health");
  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 200);
  const data = await res.json();
  assert.strictEqual(data.status, "ok");
});

test("request 7-day trial key", async () => {
  const env = createMockEnv();
  const req = new Request("https://api.trisync.dev/trial", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email: "developer@example.com" })
  });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 200);
  const data = await res.json();
  assert.strictEqual(data.ok, true);
  assert.strictEqual(data.tier, "trial");
  assert.match(data.license_key, /^TRI-[0-9A-F]{8}-[0-9A-F]{8}-[0-9A-F]{8}$/);
  assert.ok(data.expires_at);

  // Validate the newly generated trial key
  const valReq = new Request("https://api.trisync.dev/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: data.license_key })
  });

  const valRes = await worker.fetch(valReq, env);
  assert.strictEqual(valRes.status, 200);
  const valData = await valRes.json();
  assert.strictEqual(valData.valid, true);
  assert.strictEqual(valData.status, "active");
  assert.strictEqual(valData.tier, "trial");
});

test("rejects invalid trial email", async () => {
  const env = createMockEnv();
  const req = new Request("https://api.trisync.dev/trial", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email: "invalid-email" })
  });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 400);
});

test("detects expired trial key", async () => {
  const env = createMockEnv();
  const expiredKey = "TRI-11111111-22222222-33333333";
  const pastDate = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();

  await env.ISSUED_KEYS.put(
    expiredKey,
    JSON.stringify({
      email: "old@example.com",
      tier: "trial",
      created_at: new Date(Date.now() - 8 * 24 * 60 * 60 * 1000).toISOString(),
      expires_at: pastDate,
      status: "active"
    })
  );

  const valReq = new Request("https://api.trisync.dev/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: expiredKey })
  });

  const valRes = await worker.fetch(valReq, env);
  assert.strictEqual(valRes.status, 200);
  const valData = await valRes.json();
  assert.strictEqual(valData.valid, false);
  assert.strictEqual(valData.status, "expired");
});

test("returns not_found for an unknown license key", async () => {
  const req = new Request("https://api.trisync.dev/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: "TRI-11111111-22222222-33333333" })
  });

  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 404);
  assert.deepStrictEqual(await res.json(), { valid: false, status: "not_found" });
});

test("detects revoked license key", async () => {
  const env = createMockEnv();
  const revokedKey = "TRI-11111111-22222222-33333333";
  await env.ISSUED_KEYS.put(
    revokedKey,
    JSON.stringify({
      email: "revoked@example.com",
      tier: "pro",
      created_at: "2026-01-01T00:00:00.000Z",
      status: "revoked",
      revoked_reason: "Chargeback"
    })
  );

  const req = new Request("https://api.trisync.dev/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: revokedKey })
  });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 200);
  assert.deepStrictEqual(await res.json(), {
    valid: false,
    status: "revoked",
    createdAt: "2026-01-01T00:00:00.000Z",
    revokedReason: "Chargeback"
  });
});

test("returns a server error for a malformed license record", async () => {
  const env = createMockEnv();
  const malformedKey = "TRI-44444444-55555555-66666666";
  await env.ISSUED_KEYS.put(malformedKey, "{malformed");

  const req = new Request("https://api.trisync.dev/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ license_key: malformedKey })
  });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 500);
  assert.deepStrictEqual(await res.json(), { error: "invalid license record" });
});
