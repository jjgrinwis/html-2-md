# HTML-to-Markdown for AI Bots

An example of a high-performance [Spin](https://spinframework.dev/) WebAssembly HTTP component that converts HTML to Markdown for AI bot consumption. Designed to run on [Akamai Functions](https://www.akamai.com/products/serverless-computing) and integrate seamlessly with Akamai Bot Manager (BVM) to optimize content delivery for AI agents.

It's an example just to show how you can use Akamai Functions to optimize the content using [html_to_markdown_rs](https://crates.io/crates/html-to-markdown-rs) crate.

## Overview

When Akamai Bot Manager detects an AI bot, this function automatically converts HTML pages to clean, AI-optimized Markdown before serving them. The optimized content is cached at the edge, reducing origin load and improving response times for AI crawlers.

### Key Features

- 🤖 **AI Bot Optimization** - Automatic Markdown conversion when BVM detects AI bots
- ⚡ **Edge Caching** - Optimized content cached at Akamai edge servers
- 🔒 **Loop Prevention** - `x-aka-function` request header prevents infinite routing loops
- 📦 **Base64 Encoding** - Safe URL handling through request headers
- 🧹 **Clean Markdown** - Removes nav, footer, scripts, styles; optimized for AI consumption
- 🔄 **Redirect Following** - Handles up to 10 redirects with relative URL resolution
- 🚰 **Streaming Passthrough** - Non-HTML content (PDFs, images, JSON) relays straight through at any size
- 🛡️ **HTTPS Only** - Security-first approach, only fetches HTTPS URLs

## Architecture

<img width="806" height="380" alt="image" src="https://github.com/user-attachments/assets/5ab7b429-d984-4d3f-8afa-88df57270774" />

### Request Flow

**For Regular Users:**

1. Request arrives at Akamai Edge
2. BVM checks → not a bot
3. Request forwarded to origin normally (via mTLS/SiteShield if configured)
4. HTML response returned to user

**For AI Bots (First Request):**

1. AI bot requests a page, `/html` in this example.
2. **BVM Detection** (`CLIENT_REQ` stage) - Akamai Bot Manager identifies bot, sets `PMUSER_BOT` from `AK_FIREWALL_TRIGGERED_RULES`, and forwards it as an `x-detected-bot` request header
   - `AK_FIREWALL_TRIGGERED_RULES` is only populated on the edge server handling the client request, so it isn't set when re-read at a parent/peer tier. The header carries the bot ID forward instead.
3. **Bot ID Propagation** (non-`CLIENT_REQ` stages) - A second rule runs on parent/peer tiers and re-sets `PMUSER_BOT` by extracting it from the incoming `x-detected-bot` request header, since `AK_FIREWALL_TRIGGERED_RULES` isn't available there
4. **CDN Routing** - Criteria match (`PMUSER_BOT` set + path `/html` + no `x-aka-function` header):
   - Encode original URL as Base64: `https://your-domain.com/html`
   - Forward to Akamai Function with `x-origin-url` header
5. **Function Processing**:
   - Decode Base64 URL
   - Add the `x-aka-function: html2md/1.0` header
   - **Callback through CDN** to fetch content (bypasses function routing because of that header)
   - CDN forwards to origin using existing security if enabled (mTLS/SiteShield)
   - Convert HTML → Markdown
   - Return optimized content
6. **Edge Caching** - CDN caches Markdown response (2 min TTL)

**For AI Bots (Subsequent Requests):**

1. AI bot requests same `/html` page
2. BVM detects bot
3. **Cache Hit** - Optimized Markdown served from edge cache
4. No function invocation, no origin fetch

### Loop Prevention

The function adds `x-aka-function: html2md/1.0` header to all outbound requests. Your Akamai delivery configuration should check for this header and bypass function routing when present, fetching from origin normally instead. This prevents infinite loops (CDN → Function → BVM → Function).

## Akamai Delivery Configuration

Bot detection and the function trigger are split across three rules, because `AK_FIREWALL_TRIGGERED_RULES` is only reliably populated on the edge server that terminates the client request (the `CLIENT_REQ` stage) — it isn't set when later read again at a parent/peer/child tier:

1. **`Detect Bot - CLIENT_REQ stage`** - runs at the `CLIENT_REQ` stage, sets `PMUSER_BOT` from `AK_FIREWALL_TRIGGERED_RULES`, and forwards it as an `x-detected-bot` request header (overwriting any existing value, in case a client sent that header itself)
2. **`Detect Bot - NON CLIENT_REQ stage (parent/peer)`** - runs on every other stage, and re-sets `PMUSER_BOT` by extracting it from the incoming `x-detected-bot` request header instead, since `AK_FIREWALL_TRIGGERED_RULES` isn't available there
3. **`HTML-2-MD for bots`** - the routing rule, which can now match on `PMUSER_BOT` alone regardless of which tier evaluates it, since the two rules above guarantee it's populated everywhere

```json
{
  "name": "Detect Bot - CLIENT_REQ stage",
  "behaviors": [
    {
      "name": "setVariable",
      "options": {
        "variableName": "PMUSER_BOT",
        "variableValue": "{{builtin.AK_FIREWALL_TRIGGERED_RULES}}"
      }
    },
    {
      "name": "modifyOutgoingRequestHeader",
      "options": {
        "action": "MODIFY",
        "customHeaderName": "x-detected-bot",
        "newHeaderValue": "{{user.PMUSER_BOT}}",
        "avoidDuplicateHeaders": true
      }
    }
  ],
  "criteria": [
    {
      "name": "requestType",
      "options": { "matchOperator": "IS", "value": "CLIENT_REQ" }
    }
  ]
}
```

```json
{
  "name": "Detect Bot - NON CLIENT_REQ stage (parent/peer)",
  "behaviors": [
    {
      "name": "setVariable",
      "options": {
        "valueSource": "EXTRACT",
        "variableName": "PMUSER_BOT",
        "extractLocation": "CLIENT_REQUEST_HEADER",
        "headerName": "x-detected-bot"
      }
    }
  ],
  "criteria": [
    {
      "name": "requestType",
      "options": { "matchOperator": "IS_NOT", "value": "CLIENT_REQ" }
    }
  ]
}
```

### Property Manager Rule

```json
{
  "name": "HTML-2-MD for bots",
  "children": [
    {
      "name": "Hit on bot-id",
      "criteria": [
        {
          "name": "matchVariable",
          "options": {
            "variableName": "PMUSER_BOT",
            "matchOperator": "IS_ONE_OF",
            "variableValues": ["BOT-69730180", "BOT-69105154"]
          }
        }
      ],
      "criteriaMustSatisfy": "all"
    }
  ],
  "criteria": [
    {
      "name": "path",
      "options": {
        "matchOperator": "MATCHES_ONE_OF",
        "values": ["/html"]
      }
    },
    {
      "name": "requestHeader",
      "options": {
        "headerName": "x-aka-function",
        "matchOperator": "DOES_NOT_EXIST"
      }
    }
  ],
  "criteriaMustSatisfy": "all"
}
```

**Criteria Breakdown:**

1. **Bot Detection** (`PMUSER_BOT` is one of `BOT-69730180`, `BOT-69105154`)
   - Checks the `PMUSER_BOT` variable for a specific bot ID. This is reliable at every tier because the two `Detect Bot` rules above keep it populated: from `AK_FIREWALL_TRIGGERED_RULES` at `CLIENT_REQ`, and from the forwarded `x-detected-bot` header everywhere else
   - Bot ID set via BVM rules in your property configuration
   - You can create your own custom bot list and combine your own bots with known Akamai bots in 1 BOT-xxxx id.
   - The full list of known Akamai bot detection rule IDs is documented here: [Akamai Bot Detection Rule IDs](https://techdocs.akamai.com/app-api-protector/docs/bot-detn-methods-rule-ids)

2. **Path Match** (`/html`)
   - Only triggers for HTML content paths
   - Prevents function calls for assets (CSS, JS, images)
   - Customize to match your content paths

3. **No Function Header** (`x-aka-function` does not exist)
   - Ensures request is NOT from the function itself
   - Critical for loop prevention
   - Function adds this header when fetching from origin

**When ALL criteria match → Route to Akamai Function**

### Child Behaviors

#### 1. Set x-origin-url Header

```json
{
  "name": "setVariable",
  "options": {
    "variableName": "PMUSER_ORIGIN_URL",
    "variableValue": "{{builtin.AK_SCHEME}}://{{builtin.AK_HOST}}{{builtin.AK_URL}}",
    "transform": "BASE_64_URL_ENCODE"
  }
}
```

- Constructs full origin URL from incoming request
- `AK_SCHEME`: `https`
- `AK_HOST`: Request hostname (e.g., `www.example.com`)
- `AK_URL`: Full path with query string (e.g., `/products?id=123`)
- Base64 encodes the URL for safe header transmission
- Stores in `PMUSER_ORIGIN_URL` variable

```json
{
  "name": "modifyOutgoingRequestHeader",
  "options": {
    "action": "ADD",
    "customHeaderName": "x-origin-url",
    "headerValue": "{{user.PMUSER_ORIGIN_URL}}"
  }
}
```

- Adds `x-origin-url` header to function request
- Value is the Base64-encoded original URL
- Function decodes this to know what content to fetch

#### 2. Functions Origin

```json
{
  "name": "origin",
  "options": {
    "hostname": "your-app-uuid.fwf.app",
    "originType": "CUSTOMER",
    "forwardHostHeader": "ORIGIN_HOSTNAME"
  }
}
```

- Routes request to your Akamai Function
- `hostname`: Your function's unique URL
- `forwardHostHeader: ORIGIN_HOSTNAME`: Sends function hostname as Host header

```json
{
  "name": "rewriteUrl",
  "options": {
    "behavior": "REWRITE",
    "targetUrl": "/"
  }
}
```

- Rewrites all paths to `/` when calling the function
- Function uses catch-all route, doesn't need path preservation
- Original URL passed via `x-origin-url` header instead

### Caching Configuration

Add this behavior to cache the optimized Markdown responses:

```json
{
  "name": "caching",
  "options": {
    "behavior": "MAX_AGE",
    "mustRevalidate": false,
    "ttl": "2m"
  }
}
```

**Caching Details:**

- **TTL**: 2 minutes (adjust based on content freshness needs)
- **Cache Key**: Includes full URL (from `x-origin-url` header)
- **Separate Cache**: Bot requests cached separately from regular user requests

Caching matters a lot here: converting a large page is the expensive part of the request. See [PERFORMANCE.md](PERFORMANCE.md) for measured numbers.

## Build & Run

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (1.78+, **stable** — no nightly toolchain needed, see [Why spin-sdk 5.2.0](#why-spin-sdk-520-and-not-6x7x))
- [Spin CLI](https://developer.fermyon.com/spin/install)
- [Akamai Functions Plugin](https://github.com/fermyon/aka-plugin)

```bash
# Install Akamai plugin
spin plugins install aka

# Authenticate
spin aka login
```

### Local Development

```bash
# Build the Wasm component
spin build

# Run locally
spin up

# Test with Base64-encoded URL
BASE64_URL=$(echo -n "https://example.com" | base64)
curl -H "x-origin-url: $BASE64_URL" http://localhost:3000/
```

**Note:** Loop prevention needs no configuration — the function always sends `x-aka-function: html2md/1.0` on outbound requests, and your delivery config keys off that header.

### Deploy to Akamai Functions

```bash
spin aka deploy
```

Your function will be deployed and you'll receive a URL like:

```
https://your-app-uuid.fwf.app
```

## Configuration

### Allowed Outbound Hosts

**IMPORTANT:** You must configure which domains the function can fetch from in `spin.toml`:

```toml
[component.html-to-md]
source = "target/wasm32-wasip2/release/html_2_md.wasm"
allowed_outbound_hosts = ["https://your-domain.com"]
```

**Security Benefits:**

- ✅ Prevents the function from being used as an open proxy
- ✅ Blocks SSRF (Server-Side Request Forgery) attacks
- ✅ Enforced at WebAssembly runtime level by Spin
- ✅ Defense-in-depth even if CDN configuration is bypassed

**Configuration Options:**

```toml
# Single domain
allowed_outbound_hosts = ["https://www.example.com"]

# Multiple domains
allowed_outbound_hosts = [
    "https://www.example.com",
    "https://api.example.com"
]

# All subdomains
allowed_outbound_hosts = ["https://*.example.com"]

# Multiple domains with wildcards
allowed_outbound_hosts = [
    "https://*.example.com",
    "https://*.anothersite.com"
]
```

**What happens when blocked:**

If the function tries to fetch a URL not in the allowlist:

```bash
# Request
curl -H "x-origin-url: $(echo -n 'https://unauthorized.com' | base64)" https://your-function-url/

# Response (502 Bad Gateway)
{"error":"Failed to fetch: https://unauthorized.com/"}

# Spin logs show
ERROR spin_runtime_factors: Outbound network destination not allowed: https://unauthorized.com
```

**Setup Steps:**

1. Identify your domain(s) that need to be fetched
2. Update `spin.toml` with `allowed_outbound_hosts`
3. Rebuild: `spin build`
4. Deploy: `spin aka deploy`
5. Test with both allowed and blocked URLs to verify

### Markdown Conversion Options

The function uses these AI-optimized settings (in `src/lib.rs`):

```rust
ConversionOptions::builder()
    .heading_style(HeadingStyle::Atx)  // Use # headings
    .skip_images(true)                 // Remove images
    .strip_tags(vec![                  // Remove boilerplate
        "nav", "footer", "aside",
        "script", "style"
    ])
    .extract_metadata(false)
    .autolinks(true)                   // Clean URL representation
    .wrap(false)                       // No hard line wrapping
    .default_title(true)               // Always include title
    .build()
```

See [html-to-markdown-rs documentation](https://docs.rs/html-to-markdown-rs/latest/html_to_markdown_rs/) for all available options.

## API

### Request

**Headers:**

- `x-origin-url` (required): Base64 URL-encoded full HTTPS URL to fetch

**Example:**

```bash
# Encode URL
BASE64_URL=$(echo -n "https://www.example.com/page" | base64)

# Call function
curl -H "x-origin-url: $BASE64_URL" https://your-function-url/
```

### Response

**Success (200):**

```
Content-Type: text/markdown; charset=utf-8

# Page Title

Content in clean Markdown format...
```

**Non-HTML passthrough:**

If the fetched URL returns anything other than `text/html` — JSON, a PDF, an image, plain text — there is nothing to convert, so the function relays the remote response unchanged: original status code, original `content-type`, original body. No error status, and no failover rules needed in the delivery configuration.

This path is **streamed**: each chunk read from the remote is written straight to the client, so memory use stays flat regardless of file size and the client starts receiving bytes immediately (measured time-to-first-byte is the same for a 27-byte body and a 10 MB one).

Streaming does **not** exempt this path from the platform's 10 MB response cap — see [Size Limits](#size-limits).

```
Content-Type: application/json

{"message":"hello, world!"}
```

**Errors:**

- `400` - Missing/invalid `x-origin-url` header, invalid Base64, invalid UTF-8, non-HTTPS URL
- `422` - Remote returned non-2xx status, HTML exceeds the 10 MiB conversion limit, empty response body, conversion failed
- `502` - Network failure, too many redirects (>10), missing Location header

All errors return JSON:

```json
{ "error": "Error message" }
```

## Logs

The function logs key events for debugging:

```
# Successful request
[html-2-md] received request | base64: aHR0cHM6Ly9... | decoded: https://example.com
[html-2-md] html: 195 KiB  →  md: 16 KiB  |  url: https://example.com

# Errors
[html-2-md] 400 missing x-origin-url header
[html-2-md] 400 invalid Base64 encoding: not-valid!!!
[html-2-md] 400 invalid URL scheme: http (must be https)
[html-2-md] 400 invalid URL format: /html | error: RelativeUrlWithoutBase
[html-2-md] 422 remote error | url: https://example.com | remote status: 404

# Non-HTML content relayed unchanged (not an error)
[html-2-md] passthrough non-html content-type: application/pdf | 51234 bytes | https://example.com/doc
```

View logs:

```bash
# Akamai Functions logs
spin aka logs

# Local logs
tail -f .spin/logs/html-to-md_stdout.txt
```

## Testing

### Unit Tests (Hurl)

```bash
# Run Hurl test suite
hurl --test tests/html-2-md.hurl --variable base_url=http://localhost:3000
```

Test cases:

- Valid Base64-encoded URL
- Missing header (400 error)
- Invalid Base64 (400 error)
- Non-HTML content-type relayed unchanged (200 passthrough)

### Manual Testing

```bash
# Test with working URL
BASE64=$(echo -n "https://www.medemblik.nl" | base64 | tr -d '=')
curl -H "x-origin-url: $BASE64" https://your-function-url/ | head -20

# Test error handling
curl https://your-function-url/
# Expected: {"error":"Missing required header: x-origin-url"}

# Test with HTTP (should fail)
BASE64=$(echo -n "http://example.com" | base64 | tr -d '=')
curl -H "x-origin-url: $BASE64" https://your-function-url/
# Expected: {"error":"Invalid URL: x-origin-url must use https"}
```

## Security

### HTTPS Only

The function only fetches HTTPS URLs to prevent:

- Man-in-the-middle attacks
- Unencrypted data transmission
- Mixed content issues

### Loop Prevention

The `x-aka-function` header ensures the function doesn't create infinite routing loops:

1. Function adds `x-aka-function: html2md/1.0` to all outbound requests
2. CDN checks for this header in property configuration
3. If present → bypass function routing, fetch from origin
4. If not present → normal BVM detection and routing

**Important:** Your Akamai property must check for this header to prevent loops. See the "Akamai Delivery Configuration" section for the criteria setup.

### Size Limits

- Maximum **HTML** size for conversion: 10 MiB, enforced incrementally as the body arrives
- This limit exists because `html-to-markdown-rs` needs the whole document in memory at once, and prevents memory exhaustion in the WebAssembly runtime
- Non-HTML content is streamed through rather than buffered, so it costs only one chunk of memory at a time — but it is **not** exempt from the platform limit below
- If you need more, just ask your Akamai contact person to raise the limit.

**Akamai Functions caps responses at 10 MB**, whether the component buffers or streams. This is a quota and can be raised on request; it is not a hard architectural limit.

Exceeding it fails untidily rather than cleanly: the runtime returns `200` with the correct `content-type`, streams ~10 MiB, then resets the HTTP/2 stream with `INTERNAL_ERROR` (curl exit 92). The body delivered is a valid _truncated prefix_, so a client that ignores the stream reset sees a successful but incomplete file. Observed cutoffs vary between runs (10.07–10.47 MB), so the cap is applied at chunk granularity rather than at an exact byte count.

> **`spin up` does not reproduce this.** Locally the same component relays a 25 MiB PDF byte-identically with curl exit 0. Large passthrough bodies can only be tested against a deployed function — and use `curl -sS` or `--fail`, because plain `-s` silently swallows the mid-stream reset and the truncated file looks like a success.

### Header Filtering

The function strips these headers from outbound requests:

- `x-origin-url` (component-specific)
- `host` (set to target host)
- Hop-by-hop headers: `connection`, `transfer-encoding`, `te`, `trailer`, `upgrade`
- Proxy headers: `proxy-authorization`, `proxy-authenticate`
- Compression: `accept-encoding`, `keep-alive`

## Performance

### Optimization

- **Edge Caching**: 2-minute TTL reduces function invocations
- **Streaming passthrough**: non-HTML responses relay with flat memory use and no size cap
- **WebAssembly**: Near-native performance
- **Minimal Dependencies**: Fast cold starts

Measured results — including the HTML→Markdown size reduction — are in [PERFORMANCE.md](PERFORMANCE.md).

### Monitoring

Key metrics to track:

- Function invocations (via Akamai Functions dashboard)
- Cache hit ratio (via Akamai CDN reporting)
- Response times (via Akamai logs)
- Error rates (via function logs)

## Troubleshooting

### Issue: Getting 400 "Invalid Base64 encoding"

**Cause:** Akamai's `base64_encode()` strips padding characters (`=`)

**Solution:** Function automatically adds padding - ensure you're using `BASE_64_URL_ENCODE` transform in Akamai config

### Issue: Getting infinite redirect loops

**Cause:** CDN not bypassing function routing for function-initiated requests

**Solution:**

1. Verify Akamai property criteria includes the `x-aka-function` `DOES_NOT_EXIST` check
2. Ensure function routing only happens when this header is NOT present
3. Check function logs to confirm `x-aka-function: html2md/1.0` header is being sent

### Issue: Function not being called

**Cause:** Criteria not matching

**Solution:**

1. Verify `PMUSER_BOT` variable is set by BVM at the `CLIENT_REQ` stage, and that the `Detect Bot - CLIENT_REQ stage` rule forwards it as the `x-detected-bot` request header
2. If your routing rule runs at a parent/peer/child tier, verify the `Detect Bot - NON CLIENT_REQ stage (parent/peer)` rule is re-populating `PMUSER_BOT` from the `x-detected-bot` header there — `AK_FIREWALL_TRIGGERED_RULES` isn't available outside `CLIENT_REQ`
3. Check path matches your content paths
4. Ensure the request doesn't already carry an `x-aka-function` header

### Issue: Response too large error

**Cause:** HTML page exceeds 10 MiB

**Solution:**

- Implement pagination for large pages
- Use more specific selectors to extract only needed content
- Consider increasing `MAX_BODY_SIZE` constant (requires Akamai Functions tier check)
- Ask your favorite Akamai contact person to raise the limits.

## Dependencies

- [spin-sdk](https://crates.io/crates/spin-sdk) v5.2.0 - Spin framework runtime
- [html-to-markdown-rs](https://crates.io/crates/html-to-markdown-rs) v3.14 - HTML to Markdown conversion
- [url](https://crates.io/crates/url) v2 - URL parsing and validation
- [base64](https://crates.io/crates/base64) v0.23 - Base64 encoding/decoding
- [anyhow](https://crates.io/crates/anyhow) v1 - Error handling
- [futures](https://crates.io/crates/futures) v0.3 - Stream/Sink traits for the streaming passthrough path

### Why spin-sdk 5.2.0 and not 6.x/7.x

This component deliberately stays on `spin-sdk` 5.x, and on **stable Rust** — no nightly toolchain, no unstable flags.

`spin-sdk` 6.0+ is WASIp3-native. Moving to it means:

- Building for `wasm32-wasip3`, which has no prebuilt `std` on stable — it requires a nightly toolchain plus `-Z build-std`
- A manifest key (`executor = { type = "wasip3-unstable" }`) that isn't part of the published Spin manifest schema
- Reworking the request handler: `#[http_component]` → `#[http_service]`, and Request/Response become the `http` crate's types (which removes the header helpers this component uses for outbound header filtering)
- Rewriting body handling, since 6.0+ replaces byte-slice bodies with a streaming `IncomingBody` — the 10 MiB size guard and UTF-8 decode both become incremental reads

The break is at 6.0, not 7.0 — building this component's `src/lib.rs` against 6.0.0 and 7.0.0 produces the same set of compile errors.

On top of that, whether Akamai Functions currently runs a WASIp3-capable Spin runtime is undocumented. Upgrading would mean a nightly toolchain and an unstable manifest flag in exchange for no functional gain, on a platform that may not accept the result. `spin-sdk` 5.2.0 builds on stable for `wasm32-wasip2` and does everything this component needs.

The other dependencies (`html-to-markdown-rs`, `base64`, `url`, `anyhow`) are not affected by this and are kept current.

## License

MIT License - see LICENSE file for details

## Contributing

Contributions welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## Support

For issues or questions:

- GitHub Issues: [Create an issue](https://github.com/your-repo/html-2-md/issues)
- Akamai Functions: [Documentation](https://techdocs.akamai.com/akamai-functions/docs)
- Spin Framework: [Discord](https://discord.gg/AAFNfS7NGf)

---

Built with ❤️ using [Spin](https://spinframework.dev/) and [Akamai Functions](https://www.akamai.com/products/serverless-computing)
