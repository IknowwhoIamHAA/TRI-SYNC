// TRI-SYNC Cloudflare Worker — Production
// Stripe webhook → KV license storage → Resend email delivery

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // Health check
    if (url.pathname === "/health") {
      return json({ status: "ok" });
    }

    // License validation
    if (url.pathname === "/validate" && request.method === "POST") {
      return validateLicense(request, env);
    }

    // 7-day Trial issuance
    if (url.pathname === "/trial" && request.method === "POST") {
      return requestTrial(request, env);
    }

    // Stripe webhook
    if (url.pathname === "/webhook" && request.method === "POST") {
      return stripeWebhook(request, env);
    }

    return new Response("Not found", { status: 404 });
  }
};

/* -------------------------------------------------------------------------- */
/*                               LICENSE VALIDATION                           */
/* -------------------------------------------------------------------------- */

async function validateLicense(request, env) {
  const body = await request.json().catch(() => null);
  if (!body || typeof body.license_key !== "string") {
    return json({ error: "license_key required" }, 400);
  }

  const raw = await env.ISSUED_KEYS.get(body.license_key);
  if (!raw) {
    return json({ valid: false, status: "not_found" }, 404);
  }

  let record;
  try {
    record = JSON.parse(raw);
  } catch {
    return json({ valid: true, status: "active" });
  }

  if (record.status === "revoked") {
    return json({
      valid: false,
      status: "revoked",
      createdAt: record.created_at,
      revokedReason: record.revoked_reason || "Revoked"
    });
  }

  if (record.expires_at && new Date(record.expires_at).getTime() < Date.now()) {
    return json({
      valid: false,
      status: "expired",
      tier: record.tier,
      createdAt: record.created_at,
      expiresAt: record.expires_at,
      message: "License key has expired"
    });
  }

  return json({
    valid: true,
    status: "active",
    tier: record.tier,
    createdAt: record.created_at,
    expiresAt: record.expires_at || null
  });
}

/* -------------------------------------------------------------------------- */
/*                                TRIAL ISSUANCE                              */
/* -------------------------------------------------------------------------- */

async function requestTrial(request, env) {
  const body = await request.json().catch(() => null);
  if (!body || typeof body.email !== "string" || !body.email.includes("@")) {
    return json({ error: "valid email required" }, 400);
  }

  const email = body.email.trim().toLowerCase();
  const now = new Date();
  const expiresAt = new Date(now.getTime() + 7 * 24 * 60 * 60 * 1000); // 7-day trial

  const tier = "trial";
  const licenseKey = await generateLicenseKey("trial-session", email, tier);

  const licenseData = {
    email,
    tier,
    created_at: now.toISOString(),
    expires_at: expiresAt.toISOString(),
    status: "active"
  };

  // Store license in KV
  await env.ISSUED_KEYS.put(licenseKey, JSON.stringify(licenseData));

  // Store marketing contact
  await env.MARKETING_CONTACTS.put(
    email,
    JSON.stringify({
      email,
      tier,
      source: "trial_request",
      created_at: now.toISOString(),
      expires_at: expiresAt.toISOString()
    })
  );

  // Send license email if Resend integration is configured
  let emailResult = { ok: false };
  if (env.RESEND_API_KEY) {
    emailResult = await sendLicenseEmail(email, licenseKey, tier, env);
  }

  return json({
    ok: true,
    license_key: licenseKey,
    tier,
    created_at: now.toISOString(),
    expires_at: expiresAt.toISOString(),
    email_sent: emailResult.ok
  });
}

/* -------------------------------------------------------------------------- */
/*                               STRIPE WEBHOOK                               */
/* -------------------------------------------------------------------------- */

async function stripeWebhook(request, env) {
  const rawBody = await request.text();
  const signature = request.headers.get("Stripe-Signature");

  if (!signature || !env.STRIPE_WEBHOOK_SECRET) {
    return json({ error: "missing Stripe signature or secret" }, 400);
  }

  // Production signature verification
  const valid = await verifyStripeSignature(rawBody, signature, env.STRIPE_WEBHOOK_SECRET);
  if (!valid) {
    return json({ error: "invalid Stripe signature" }, 400);
  }

  let event;
  try {
    event = JSON.parse(rawBody);
  } catch {
    return json({ error: "invalid JSON" }, 400);
  }

  if (event.type !== "checkout.session.completed") {
    return json({ received: true, ignored: true });
  }

  const session = event.data.object;
  const email = session.customer_details?.email;
  const tier = session.metadata?.tier || "unknown";

  if (!email) {
    return json({ error: "missing customer email" }, 400);
  }

  // Generate deterministic license key
  const licenseKey = await generateLicenseKey(session.id, email, tier);

  // Store license in KV
  await env.ISSUED_KEYS.put(
    licenseKey,
    JSON.stringify({
      email,
      tier,
      created_at: new Date().toISOString()
    })
  );

  // Store marketing contact
  await env.MARKETING_CONTACTS.put(
    email,
    JSON.stringify({
      email,
      tier,
      source: "stripe_checkout",
      created_at: new Date().toISOString()
    })
  );

  // Send license email
  const emailResult = await sendLicenseEmail(email, licenseKey, tier, env);

  return json({
    ok: true,
    license_key: licenseKey,
    email_sent: emailResult.ok
  });
}

/* -------------------------------------------------------------------------- */
/*                          STRIPE SIGNATURE VERIFICATION                     */
/* -------------------------------------------------------------------------- */

async function verifyStripeSignature(payload, signature, secret) {
  // Stripe signature format: t=timestamp,v1=signature
  const parts = Object.fromEntries(
    signature.split(",").map(s => s.split("=", 2))
  );

  const signedPayload = `${parts.t}.${payload}`;
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );

  const signatureBytes = await crypto.subtle.sign(
    "HMAC",
    key,
    new TextEncoder().encode(signedPayload)
  );

  const expected = hex(signatureBytes);
  return expected === parts.v1;
}

/* -------------------------------------------------------------------------- */
/*                             LICENSE KEY GENERATION                         */
/* -------------------------------------------------------------------------- */

async function generateLicenseKey(sessionId, email, tier) {
  const data = `${sessionId}:${email}:${tier}:${Date.now()}`;
  const hashBytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(data));
  const h = hex(hashBytes).toUpperCase();
  return `TRI-${h.slice(0, 8)}-${h.slice(8, 16)}-${h.slice(16, 24)}`;
}

/* -------------------------------------------------------------------------- */
/*                               RESEND EMAIL SEND                            */
/* -------------------------------------------------------------------------- */

async function sendLicenseEmail(email, licenseKey, tier, env) {
  const payload = {
    from: "TRI-SYNC <support@trisync.dev>",
    to: email,
    subject: "Your TRI-SYNC License Key",
    html: `
      <p>Hi,</p>
      <p>Thank you for subscribing to TRI-SYNC (${tier}).</p>
      <p>Your license key:</p>
      <pre>${licenseKey}</pre>
      <p>Use this key to activate your TRI-SYNC runtime.</p>
      <p>— TRI-SYNC</p>
    `
  };

  const res = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${env.RESEND_API_KEY}`,
      "Content-Type": "application/json"
    },
    body: JSON.stringify(payload)
  });

  return { ok: res.ok };
}

/* -------------------------------------------------------------------------- */
/*                                   HELPERS                                  */
/* -------------------------------------------------------------------------- */

function json(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" }
  });
}

function hex(buffer) {
  return [...new Uint8Array(buffer)]
    .map(b => b.toString(16).padStart(2, "0"))
    .join("");
}

