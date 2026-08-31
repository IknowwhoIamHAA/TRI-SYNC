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

  const stored = await env.ISSUED_KEYS.get(body.license_key);
  return json({ valid: !!stored });
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
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(data));
  return `TS-${hex(hash).slice(0, 20).toUpperCase()}`;
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

