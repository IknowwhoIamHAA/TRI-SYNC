import assert from "node:assert/strict";
import test from "node:test";
import worker from "../src/index.js";

function createMockEnv() {
  const issued = new Map();
  const marketing = new Map();

  return {
   stores: { issued, marketing },
   ISSUED_KEYS: {
     get: async (key) => issued.get(key) ?? null,
     put: async (key, value) => {
       issued.set(key, value);
     }
   },
   MARKETING_CONTACTS: {
     get: async (key) => marketing.get(key) ?? null,
     put: async (key, value) => {
       marketing.set(key, value);
     }
   }
  };
}

function createJsonRequest(path, body, init = {}) {
  const headers = new Headers(init.headers);
  headers.set("Content-Type", "application/json");

  return new Request(`https://api.trisync.dev${path}`, {
   method: "POST",
   ...init,
   headers,
   body: JSON.stringify(body)
  });
}

async function readJson(response) {
  return response.json();
}

test("createJsonRequest preserves JSON content type with custom headers", () => {
  const req = createJsonRequest("/trial", { email: "developer@example.com" }, {
    headers: { Authorization: "******" }
  });

  assert.strictEqual(req.headers.get("content-type"), "application/json");
  assert.strictEqual(req.headers.get("authorization"), "******");
});

test("health endpoint", async () => {
  const req = new Request("https://api.trisync.dev/health");
  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 200);
  assert.strictEqual(res.headers.get("access-control-allow-origin"), "https://www.trisync.dev");
  const data = await readJson(res);
  assert.strictEqual(data.status, "ok");
});

test("accepts the trial request CORS preflight", async () => {
  const req = new Request("https://api.trisync.dev/trial", {
    method: "OPTIONS",
    headers: {
      Origin: "https://www.trisync.dev",
      "Access-Control-Request-Method": "POST",
      "Access-Control-Request-Headers": "content-type"
    }
  });
  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 204);
  assert.strictEqual(res.headers.get("access-control-allow-origin"), "https://www.trisync.dev");
  assert.strictEqual(res.headers.get("access-control-allow-methods"), "POST, OPTIONS");
  assert.strictEqual(res.headers.get("access-control-allow-headers"), "Content-Type");
});

test("request 7-day trial key", async () => {
  const env = createMockEnv();
  const req = createJsonRequest("/trial", { email: " Developer@Example.com " });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 200);
  const data = await readJson(res);
  assert.strictEqual(data.ok, true);
  assert.strictEqual(data.tier, "trial");
  assert.match(data.license_key, /^TRI-[0-9A-F]{8}-[0-9A-F]{8}-[0-9A-F]{8}$/);
  assert.ok(data.expires_at);
  assert.strictEqual(data.email_sent, false);

  const issuedRecord = JSON.parse(env.stores.issued.get(data.license_key));
  assert.deepStrictEqual(issuedRecord.email, "developer@example.com");
  assert.deepStrictEqual(issuedRecord.tier, "trial");
  assert.deepStrictEqual(issuedRecord.status, "active");

  const marketingRecord = JSON.parse(env.stores.marketing.get("developer@example.com"));
  assert.deepStrictEqual(marketingRecord, {
    email: "developer@example.com",
    tier: "trial",
    source: "trial_request",
    created_at: issuedRecord.created_at,
    expires_at: issuedRecord.expires_at
  });
  assert.strictEqual(marketingRecord.expires_at, data.expires_at);

  // Validate the newly generated trial key
  const valReq = createJsonRequest("/validate", { license_key: data.license_key });

  const valRes = await worker.fetch(valReq, env);
  assert.strictEqual(valRes.status, 200);
  const valData = await readJson(valRes);
  assert.strictEqual(valData.valid, true);
  assert.strictEqual(valData.status, "active");
  assert.strictEqual(valData.tier, "trial");
});

test("rejects invalid trial email", async () => {
  const env = createMockEnv();
  const req = createJsonRequest("/trial", { email: "invalid-email" });

  const res = await worker.fetch(req, env);
  assert.strictEqual(res.status, 400);
  assert.deepStrictEqual(await readJson(res), { error: "valid email required" });
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

  const valReq = createJsonRequest("/validate", { license_key: expiredKey });

  const valRes = await worker.fetch(valReq, env);
  assert.strictEqual(valRes.status, 200);
  const valData = await readJson(valRes);
  assert.strictEqual(valData.valid, false);
  assert.strictEqual(valData.status, "expired");
});

test("rejects missing license_key payload", async () => {
  const req = createJsonRequest("/validate", {});

  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 400);
  assert.deepStrictEqual(await readJson(res), { error: "license_key required" });
});

test("returns not_found for unknown license keys", async () => {
  const req = createJsonRequest("/validate", {
    license_key: "TRI-AAAAAAAA-BBBBBBBB-CCCCCCCC"
  });

  const res = await worker.fetch(req, createMockEnv());
  assert.strictEqual(res.status, 404);
  assert.deepStrictEqual(await readJson(res), {
    valid: false,
    status: "not_found"
  });
});
