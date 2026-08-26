/**
 * TRI-SYNC Cloudflare Worker — Stripe → KV → Resend Automations
 *
 * Receives Stripe webhook events, issues deterministic TRI-SYNC license keys,
 * persists them to Cloudflare KV, and triggers Resend Automations for delivery.
 *
 * Required environment bindings (set in wrangler.toml or the Cloudflare dashboard):
 *   STRIPE_API_KEY          — Stripe secret key (sk_live_... or sk_test_...)
 *   STRIPE_WEBHOOK_SECRET   — Base64-encoded Stripe webhook signing secret
 *   RESEND_API_KEY          — Resend API key
 *   RESEND_AUTOMATION_ID    — Resend Automation ID to trigger
 *   ISSUED_KEYS             — KV namespace binding for persisting issued license keys
 */

import Stripe from 'stripe';

export default {
  /**
   * @param {Request} request
   * @param {Env} env
   * @returns {Promise<Response>}
   */
  async fetch(request, env) {
    if (request.method !== 'POST') {
      return new Response('Method Not Allowed', { status: 405 });
    }

    const stripe = new Stripe(env.STRIPE_API_KEY, { apiVersion: '2024-04-10' });

    // Read the raw body once — required for signature verification.
    const body = await request.text();
    const signature = request.headers.get('stripe-signature');

    if (!signature) {
      return new Response('Missing stripe-signature header', { status: 400 });
    }

    // STRIPE_WEBHOOK_SECRET is stored Base64-encoded to avoid issues with the
    // "whsec_" prefix that can be misinterpreted by some secret managers.
    let webhookSecret;
    try {
      webhookSecret = atob(env.STRIPE_WEBHOOK_SECRET);
    } catch {
      return new Response('Misconfigured webhook secret', { status: 500 });
    }

    let event;
    try {
      event = await stripe.webhooks.constructEventAsync(body, signature, webhookSecret);
    } catch (err) {
      return new Response(`Webhook signature verification failed: ${err.message}`, { status: 400 });
    }

    if (event.type === 'checkout.session.completed') {
      const session = event.data.object;

      const customerEmail = session.customer_details?.email ?? '';
      const customerId = session.customer ?? '';
      const paymentIntentId = session.payment_intent ?? '';
      const plan = session.metadata?.plan ?? 'unknown';

      // Generate a deterministic TRI-SYNC license key that mirrors the format
      // produced by the Rust license module: "TS-" + first 16 hex chars (uppercase)
      // of SHA-256(email:customerId).
      const licenseKey = await generateLicenseKey(customerEmail, customerId);

      // Canonical KV record — fields sorted lexicographically, numeric timestamps
      // stored as integers (not strings) for deterministic replay.
      const record = canonicalRecord({
        created: Date.now(),
        customerId,
        email: customerEmail,
        paymentIntentId,
        plan,
      });

      await env.ISSUED_KEYS.put(licenseKey, record);

      try {
        await triggerResendAutomation(env, {
          email: customerEmail,
          license_key: licenseKey,
          name: session.customer_details?.name ?? '',
          plan,
          stripe_customer_id: customerId,
          stripe_payment_intent: paymentIntentId,
        });
      } catch (err) {
        // Return a 500 so Stripe retries the webhook. The KV write is
        // idempotent (same licenseKey → same value), so retries are safe.
        return new Response(`Resend Automation failed: ${err.message}`, { status: 500 });
      }
    }

    return new Response('OK', { status: 200 });
  },
};

/**
 * Generate a deterministic TRI-SYNC license key.
 *
 * Algorithm: SHA-256(email + ":" + customerId), take the first 16 hex
 * characters (uppercase), prefix with "TS-".
 *
 * This mirrors the Rust implementation in src/license.rs so that keys issued
 * by the worker can be validated offline by the Rust binary without any
 * additional lookup.
 *
 * @param {string} email
 * @param {string} customerId
 * @returns {Promise<string>}
 */
async function generateLicenseKey(email, customerId) {
  const data = `${email}:${customerId}`;
  const encoded = new TextEncoder().encode(data);
  const hashBuffer = await crypto.subtle.digest('SHA-256', encoded);
  const hex = [...new Uint8Array(hashBuffer)]
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
  return `TS-${hex.slice(0, 16).toUpperCase()}`;
}

/**
 * Serialize a plain object to canonical JSON suitable for deterministic replay.
 *
 * Rules (TRI-SYNC SPEC §5):
 *  - Object keys are sorted lexicographically by raw UTF-8 byte order.
 *  - No trailing whitespace.
 *  - Numbers are encoded without unnecessary precision (native JSON.stringify
 *    already satisfies this for integers).
 *
 * @param {Record<string, unknown>} obj
 * @returns {string}
 */
function canonicalRecord(obj) {
  const sorted = Object.fromEntries(
    Object.entries(obj).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  );
  return JSON.stringify(sorted);
}

/**
 * Trigger a Resend Automation with the given payload.
 *
 * The payload fields are sorted for a stable canonical request body.
 *
 * @param {Env} env
 * @param {Record<string, string>} payload
 * @returns {Promise<void>}
 */
async function triggerResendAutomation(env, payload) {
  const automationId = env.RESEND_AUTOMATION_ID;
  const body = canonicalRecord(payload);

  const response = await fetch(
    `https://api.resend.com/automations/${encodeURIComponent(automationId)}/trigger`,
    {
      method: 'POST',
      headers: {
        Authorization: 'Bearer ' + env.RESEND_API_KEY,
        'Content-Type': 'application/json',
      },
      body,
    }
  );

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(`Resend Automation trigger failed (${response.status}): ${text}`);
  }
}
