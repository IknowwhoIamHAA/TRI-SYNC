/**
 * TRI-SYNC Cloudflare Worker
 *
 * Routes:
 *   GET  /health           — Health check; returns {"status":"ok"}
 *   POST /webhook          — Stripe webhook receiver (HMAC-SHA256 verified)
 *   GET  /validate         — License key lookup: ?key=TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX
 *   POST /revoke           — Admin: revoke a license by key (requires ADMIN_SECRET header)
 *   POST /marketing/subscribe — Marketing waitlist sign-up (requires MARKETING_API_KEY)
 *
 * Required secrets (wrangler secret put ... --env production):
 *   STRIPE_WEBHOOK_SECRET  — Raw Stripe webhook signing secret (whsec_... value)
 *   STRIPE_API_KEY         — Stripe secret key for outbound Stripe API calls
 *   RESEND_API_KEY         — Resend API key for transactional email
 *   RESEND_FROM_EMAIL      — Verified sender address (e.g. "TRI-SYNC <noreply@trisync.dev>")
 *   ADMIN_SECRET           — Shared secret for admin endpoints (POST /revoke)
 *   MARKETING_API_KEY      — ****** required by POST /marketing/subscribe
 *
 * Required KV namespace bindings (wrangler.toml):
 *   ISSUED_KEYS            — Stores issued license records, indexed by session and key
 *   MARKETING_CONTACTS     — Stores marketing waitlist / subscriber records
 */

// ---------------------------------------------------------------------------
// Rate-limit configuration for GET /validate
// ---------------------------------------------------------------------------
/** Maximum validate requests allowed per IP within the window. */
const RATE_LIMIT_MAX = 20;
/** Rolling window duration in seconds. */
const RATE_LIMIT_WINDOW_S = 60;

export default {
  /**
   * @param {Request} request
   * @param {Env} env
   * @param {ExecutionContext} ctx
   * @returns {Promise<Response>}
   */
  async fetch(request, env, ctx) {
    const url = new URL(request.url);

    if (url.pathname === '/health') {
      return handleHealth();
    }

    if (url.pathname === '/validate') {
      return handleValidate(request, env);
    }

    if (url.pathname === '/revoke') {
      return handleRevoke(request, env);
    }

    if (url.pathname.startsWith('/marketing/')) {
      return handleMarketing(request, env, url);
    }

    if (url.pathname !== '/webhook') {
      return new Response('Not Found', { status: 404 });
    }

    if (request.method !== 'POST') {
      return new Response('Method Not Allowed', { status: 405 });
    }

    const bodyText = await request.text();
    const sigHeader = request.headers.get('stripe-signature');

    if (!sigHeader) {
      console.error('Missing stripe-signature header');
      return new Response('Bad Request', { status: 400 });
    }

    let event;
    try {
      event = await verifyStripeSignature(bodyText, sigHeader, env.STRIPE_WEBHOOK_SECRET);
    } catch (err) {
      console.error('Signature verification failed:', err);
      return new Response('Signature verification failed', { status: 400 });
    }

    console.log('Stripe event:', event.type);

    try {
      switch (event.type) {
        case 'checkout.session.completed':
          await handleCheckoutCompleted(event, env, ctx);
          break;

        case 'checkout.session.expired':
          await handleCheckoutExpired(event, env);
          break;

        case 'payment_intent.payment_failed':
          await handlePaymentFailed(event, env);
          break;

        case 'customer.subscription.deleted':
          await handleSubscriptionDeleted(event, env);
          break;

        case 'invoice.payment_succeeded':
          console.log('Payment succeeded:', event.data.object.id);
          break;

        case 'customer.subscription.updated':
          console.log('Subscription updated:', event.data.object.id);
          break;

        default:
          console.log('Unhandled event type:', event.type);
      }
    } catch (err) {
      console.error('Handler error:', err);
      // Return 200 so Stripe does not retry events that reached handler logic.
      return new Response('Webhook handler error', { status: 200 });
    }

    return new Response('OK', { status: 200 });
  },
};

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/**
 * GET /health
 *
 * Returns a simple health check response. Used by Cloudflare uptime monitors
 * and load balancers.
 *
 * @returns {Response}
 */
function handleHealth() {
  return new Response(JSON.stringify({ status: 'ok' }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

/**
 * POST /marketing/subscribe
 *
 * Records an email address in the MARKETING_CONTACTS KV namespace.
 * Requires the `X-Marketing-Api-Key` header to match the MARKETING_API_KEY secret.
 *
 * Request body (JSON): { "email": "user@example.com", "source": "landing-page" }
 * Response: 200 OK with { "ok": true } on success, or 400/401/409.
 *
 * @param {Request} request
 * @param {Env} env
 * @param {URL} url
 * @returns {Promise<Response>}
 */
async function handleMarketing(request, env, url) {
  const cors = {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Methods': 'POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type, X-Marketing-Api-Key',
  };

  if (request.method === 'OPTIONS') {
    return new Response(null, { status: 204, headers: cors });
  }

  if (url.pathname !== '/marketing/subscribe') {
    return new Response('Not Found', { status: 404, headers: cors });
  }

  if (request.method !== 'POST') {
    return new Response('Method Not Allowed', { status: 405, headers: cors });
  }

  // Authenticate with MARKETING_API_KEY.
  const apiKey = request.headers.get('X-Marketing-Api-Key') || '';
  if (!env.MARKETING_API_KEY || !secureCompare(apiKey, env.MARKETING_API_KEY)) {
    return new Response('Unauthorized', { status: 401, headers: cors });
  }

  let body;
  try {
    body = await request.json();
  } catch {
    return new Response('Invalid JSON body', { status: 400, headers: cors });
  }

  const email = (body.email || '').trim().toLowerCase();
  if (!email || !email.includes('@')) {
    return new Response('Invalid email', { status: 400, headers: cors });
  }

  const source = (body.source || 'unknown').slice(0, 64);
  const existing = await env.MARKETING_CONTACTS.get('contact:' + email);
  if (existing) {
    return new Response(JSON.stringify({ ok: true, already_subscribed: true }), {
      status: 200,
      headers: { 'Content-Type': 'application/json', ...cors },
    });
  }

  const record = canonicalRecord({
    email,
    source,
    subscribedAt: new Date().toISOString(),
  });

  await env.MARKETING_CONTACTS.put('contact:' + email, record);
  console.log('Marketing contact stored:', email, 'source:', source);

  return new Response(JSON.stringify({ ok: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json', ...cors },
  });
}

/**
 * GET /validate?key=TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX
 *
 * Returns the license metadata JSON for a valid key, or 404.
 * Rate-limited to RATE_LIMIT_MAX requests per IP per RATE_LIMIT_WINDOW_S seconds
 * to prevent enumeration attacks.
 *
 * @param {Request} request
 * @param {Env} env
 * @returns {Promise<Response>}
 */
async function handleValidate(request, env) {
  if (request.method !== 'GET') {
    return new Response('Method Not Allowed', { status: 405 });
  }

  // Rate limit by connecting IP.
  const ip = request.headers.get('CF-Connecting-IP') || 'unknown';
  const window = Math.floor(Date.now() / 1000 / RATE_LIMIT_WINDOW_S);
  const rlKey = 'rl:' + ip + ':' + window;

  const countStr = await env.ISSUED_KEYS.get(rlKey);
  const count = countStr ? parseInt(countStr, 10) : 0;

  if (count >= RATE_LIMIT_MAX) {
    return new Response('Too Many Requests', { status: 429 });
  }

  // Increment counter; TTL set to 2× the window so stale entries are
  // eventually cleaned up even if the window boundary is crossed mid-request.
  await env.ISSUED_KEYS.put(rlKey, String(count + 1), {
    expirationTtl: RATE_LIMIT_WINDOW_S * 2,
  });

  const key = new URL(request.url).searchParams.get('key');
  if (!key) {
    return new Response('Missing key parameter', { status: 400 });
  }

  // Reverse-lookup: bykey:<licenseKey> → sessionId
  const sessionId = await env.ISSUED_KEYS.get('bykey:' + key);
  if (!sessionId) {
    return new Response('Not Found', { status: 404 });
  }

  const record = await env.ISSUED_KEYS.get('license:' + sessionId);
  if (!record) {
    return new Response('Not Found', { status: 404 });
  }

  return new Response(record, {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

/**
 * POST /revoke
 *
 * Admin endpoint to programmatically revoke a license key (refunds, chargebacks,
 * manual revocations that don't come through Stripe events).
 *
 * Authentication: the Authorization request header must contain the ADMIN_SECRET
 * value as a bearer token. The token is compared in constant time to prevent
 * timing attacks.
 *
 * Request body (JSON): { "key": "TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX", "reason": "..." }
 * Response: 200 OK with updated record JSON, or 400/401/404.
 *
 * @param {Request} request
 * @param {Env} env
 * @returns {Promise<Response>}
 */
async function handleRevoke(request, env) {
  if (request.method !== 'POST') {
    return new Response('Method Not Allowed', { status: 405 });
  }

  // Authentication
  const authHeader = request.headers.get('Authorization') || '';
  const token = authHeader.startsWith('Bearer ') ? authHeader.slice(7) : '';
  if (!env.ADMIN_SECRET || !secureCompare(token, env.ADMIN_SECRET)) {
    return new Response('Unauthorized', { status: 401 });
  }

  let body;
  try {
    body = await request.json();
  } catch {
    return new Response('Invalid JSON body', { status: 400 });
  }

  const { key, reason } = body;
  if (!key) {
    return new Response('Missing key field', { status: 400 });
  }

  const sessionId = await env.ISSUED_KEYS.get('bykey:' + key);
  if (!sessionId) {
    return new Response('Not Found', { status: 404 });
  }

  const raw = await env.ISSUED_KEYS.get('license:' + sessionId);
  if (!raw) {
    return new Response('Not Found', { status: 404 });
  }

  let rec;
  try {
    rec = JSON.parse(raw);
  } catch {
    return new Response('Corrupt KV record', { status: 500 });
  }

  rec.status = 'revoked';
  rec.revokedAt = new Date().toISOString();
  rec.revokedReason = reason || 'admin_revoke';

  const updated = canonicalRecord(rec);
  await env.ISSUED_KEYS.put('license:' + sessionId, updated);
  console.log('License revoked via /revoke for key:', key, 'reason:', rec.revokedReason);

  return new Response(updated, {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

/**
 * Handle `checkout.session.completed`.
 *
 * Idempotent: if a license for this session already exists the handler returns
 * early without re-sending the email.
 *
 * Secondary indexes written:
 *   bykey:<licenseKey>      → sessionId
 *   bycustomer:<customerId> → sessionId  (used by subscription deletion handler)
 *
 * @param {object} event
 * @param {Env} env
 * @param {ExecutionContext} ctx
 */
async function handleCheckoutCompleted(event, env, ctx) {
  const session = event.data.object;

  const sessionId = session.id;
  const customerEmail = session.customer_details?.email || session.customer_email;
  const customerId = session.customer;

  if (!sessionId) throw new Error('Missing session.id');
  if (!customerEmail) throw new Error('Missing customer email');

  // Idempotency: skip if a license was already issued for this session.
  const existing = await env.ISSUED_KEYS.get('license:' + sessionId);
  if (existing) {
    console.log('License already issued for session:', sessionId);
    return;
  }

  const licenseKey = await generateLicenseKey(sessionId, customerEmail);

  const record = canonicalRecord({
    createdAt: new Date().toISOString(),
    customerId: customerId || '',
    email: customerEmail,
    licenseKey,
    sessionId,
    status: 'active',
  });

  // Write primary record and all secondary indexes.
  const writes = [
    env.ISSUED_KEYS.put('license:' + sessionId, record),
    env.ISSUED_KEYS.put('bykey:' + licenseKey, sessionId),
  ];
  if (customerId) {
    writes.push(env.ISSUED_KEYS.put('bycustomer:' + customerId, sessionId));
  }
  await Promise.all(writes);

  console.log('License stored in KV for session:', sessionId);

  const html =
    '<p>Hi,</p>' +
    '<p>Thank you for purchasing TRI-SYNC.</p>' +
    '<p>Your license key:</p>' +
    '<pre>' + licenseKey + '</pre>' +
    '<p>Keep this key safe. You will need it to activate TRI-SYNC.</p>' +
    '<p>Activate with:</p>' +
    '<pre>export TRISYNC_LICENSE_KEY=' + licenseKey + '\ntri-sync verify --log events.jsonl</pre>';

  await sendEmail(env, {
    to: customerEmail,
    subject: 'Your TRI-SYNC License',
    html,
  });

  console.log('License email sent to:', customerEmail);
}

/**
 * Handle `checkout.session.expired`.
 *
 * Marks any existing license record as revoked.
 *
 * @param {object} event
 * @param {Env} env
 */
async function handleCheckoutExpired(event, env) {
  const sessionId = event.data.object.id;
  if (!sessionId) {
    console.warn('checkout.session.expired: missing session.id');
    return;
  }
  await revokeBySession(env, sessionId, 'checkout.session.expired');
}

/**
 * Handle `payment_intent.payment_failed`.
 *
 * Marks the associated license record (if any) as revoked.
 *
 * @param {object} event
 * @param {Env} env
 */
async function handlePaymentFailed(event, env) {
  const intent = event.data.object;
  const sessionId = intent.metadata?.checkout_session;
  if (!sessionId) {
    console.log('payment_intent.payment_failed: no checkout_session metadata, skipping');
    return;
  }
  await revokeBySession(env, sessionId, 'payment_intent.payment_failed');
}

/**
 * Handle `customer.subscription.deleted`.
 *
 * Looks up the license associated with the Stripe customer via the
 * `bycustomer:<customerId>` secondary index written at checkout, and revokes it.
 *
 * @param {object} event
 * @param {Env} env
 */
async function handleSubscriptionDeleted(event, env) {
  const subscription = event.data.object;
  const customerId = subscription.customer;

  if (!customerId) {
    console.warn('customer.subscription.deleted: missing customer id');
    return;
  }

  const sessionId = await env.ISSUED_KEYS.get('bycustomer:' + customerId);
  if (!sessionId) {
    console.log('customer.subscription.deleted: no license for customer', customerId);
    return;
  }

  await revokeBySession(env, sessionId, 'customer.subscription.deleted');
}

// ---------------------------------------------------------------------------
// Shared revocation helper
// ---------------------------------------------------------------------------

/**
 * Mark a license record as revoked.  No-ops silently when no record exists.
 *
 * @param {Env} env
 * @param {string} sessionId
 * @param {string} reason
 */
async function revokeBySession(env, sessionId, reason) {
  const raw = await env.ISSUED_KEYS.get('license:' + sessionId);
  if (!raw) {
    console.log(reason + ': no license record for session', sessionId);
    return;
  }

  let rec;
  try {
    rec = JSON.parse(raw);
  } catch {
    console.error(reason + ': corrupt KV record for session', sessionId);
    return;
  }

  rec.status = 'revoked';
  rec.revokedAt = new Date().toISOString();
  rec.revokedReason = reason;

  await env.ISSUED_KEYS.put('license:' + sessionId, canonicalRecord(rec));
  console.log('License revoked (' + reason + ') for session:', sessionId);
}

// ---------------------------------------------------------------------------
// Stripe signature verification (no SDK — uses Web Crypto HMAC-SHA256)
// ---------------------------------------------------------------------------

/**
 * Verify a Stripe webhook signature and return the parsed event object.
 *
 * signed_payload = timestamp + "." + payload
 * expected       = HMAC-SHA256(secret, signed_payload)
 *
 * @param {string} payload    Raw request body (string, not parsed).
 * @param {string} sigHeader  Value of the `stripe-signature` header.
 * @param {string} secret     Raw webhook signing secret (whsec_... value).
 * @returns {Promise<object>} Parsed Stripe event.
 */
async function verifyStripeSignature(payload, sigHeader, secret) {
  const parts = sigHeader.split(',').reduce((acc, part) => {
    const eq = part.indexOf('=');
    if (eq !== -1) acc[part.slice(0, eq)] = part.slice(eq + 1);
    return acc;
  }, {});

  const timestamp = parts['t'];
  const signature = parts['v1'];

  if (!timestamp || !signature) {
    throw new Error('Invalid stripe-signature header');
  }

  const age = Math.floor(Date.now() / 1000) - parseInt(timestamp, 10);
  if (Math.abs(age) > 300) {
    throw new Error('Timestamp outside tolerance window');
  }

  const encoder = new TextEncoder();
  const signedPayload = timestamp + '.' + payload;
  const key = await crypto.subtle.importKey(
    'raw',
    encoder.encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign']
  );
  const expectedBuf = await crypto.subtle.sign('HMAC', key, encoder.encode(signedPayload));
  const expected = Array.from(new Uint8Array(expectedBuf))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');

  if (!secureCompare(expected, signature)) {
    throw new Error('Invalid signature');
  }

  return JSON.parse(payload);
}

/**
 * Constant-time string comparison to prevent timing attacks.
 *
 * Always iterates the full length of `a` (the locally-computed value).
 * A length mismatch is folded into the result without short-circuiting.
 *
 * @param {string} a
 * @param {string} b
 * @returns {boolean}
 */
function secureCompare(a, b) {
  let result = a.length === b.length ? 0 : 1;
  for (let i = 0; i < a.length; i++) {
    result |= a.charCodeAt(i) ^ (b.charCodeAt(i) || 0);
  }
  return result === 0;
}

// ---------------------------------------------------------------------------
// License key generation
// ---------------------------------------------------------------------------

/**
 * Generate a deterministic TRI-SYNC license key.
 *
 * Format:    TRI-XXXXXXXX-XXXXXXXX-XXXXXXXX  (3 × 8 uppercase hex chars)
 * Algorithm: SHA-256(sessionId + ":" + email), first 24 hex chars.
 *
 * @param {string} sessionId  Stripe Checkout Session ID.
 * @param {string} email      Customer email address.
 * @returns {Promise<string>}
 */
async function generateLicenseKey(sessionId, email) {
  const data = sessionId + ':' + email;
  const encoder = new TextEncoder();
  const hashBuf = await crypto.subtle.digest('SHA-256', encoder.encode(data));
  const hash = Array.from(new Uint8Array(hashBuf))
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
  return (
    'TRI-' +
    hash.slice(0, 8).toUpperCase() + '-' +
    hash.slice(8, 16).toUpperCase() + '-' +
    hash.slice(16, 24).toUpperCase()
  );
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/**
 * Serialize a plain object to canonical JSON (TRI-SYNC SPEC §5).
 *
 * Object keys are sorted lexicographically. Numeric values are not re-encoded.
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
 * Send a transactional email via Resend.
 *
 * @param {Env} env
 * @param {{ to: string, subject: string, html: string }} opts
 * @returns {Promise<void>}
 */
async function sendEmail(env, { to, subject, html }) {
  const body = canonicalRecord({
    from: env.RESEND_FROM_EMAIL,
    html,
    subject,
    to,
  });

  const res = await fetch('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      Authorization: 'Bearer ' + env.RESEND_API_KEY,
      'Content-Type': 'application/json',
    },
    body,
  });

  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error('Resend email failed (' + res.status + '): ' + text);
  }
}
