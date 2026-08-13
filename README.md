# HTML-to-Markdown for AI Bots

An example of a high-performance [Spin](https://spinframework.dev/) WebAssembly HTTP component that converts HTML to Markdown for AI bot consumption. Designed to run on [Akamai Functions](https://www.akamai.com/products/serverless-computing) and integrate seamlessly with Akamai Bot Manager (BVM) to optimize content delivery for AI agents.

It's an example just to show how you can use Akamai Functions to optimize the content using [html_to_markdown_rs](https://crates.io/crates/html-to-markdown-rs) crate.

## Overview

When Akamai Bot Manager detects an AI bot, this function automatically converts HTML pages to clean, AI-optimized Markdown before serving them. The optimized content is cached at the edge, reducing origin load and improving response times for AI crawlers.

### Key Features

- 🤖 **AI Bot Optimization** - Automatic Markdown conversion when BVM detects AI bots
- ⚡ **Edge Caching** - Optimized content cached at Akamai edge servers
- 🔒 **Loop Prevention** - Secure BVM bypass mechanism prevents infinite routing loops
- 📦 **Base64 Encoding** - Safe URL handling through request headers
- 🧹 **Clean Markdown** - Removes nav, footer, scripts, styles; optimized for AI consumption
- 🔄 **Redirect Following** - Handles up to 10 redirects with relative URL resolution
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

1. AI bot requests `/html` from your domain
2. **BVM Detection** - Akamai Bot Manager identifies bot (sets `PMUSER_BOT = "bot-123"`)
3. **CDN Routing** - Criteria match (bot detected + path `/html` + no bypass key):
   - Encode original URL as Base64: `https://your-domain.com/html`
   - Forward to Akamai Function with `x-origin-url` header
4. **Function Processing**:
   - Decode Base64 URL
   - Add `x-bvm-bypass-key` and `x-aka-function` headers
   - **Callback through CDN** to fetch content (bypasses function routing due to bypass key)
   - CDN forwards to origin using existing security if enabled (mTLS/SiteShield)
   - Convert HTML → Markdown
   - Return optimized content
5. **Edge Caching** - CDN caches Markdown response (5 min TTL, prefresh at 4 min)

**For AI Bots (Subsequent Requests):**

1. AI bot requests same `/html` page
2. BVM detects bot
3. **Cache Hit** - Optimized Markdown served from edge cache
4. No function invocation, no origin fetch

### Loop Prevention

The function adds `x-bvm-bypass-key: production-secure-key-change-me` header to outbound requests. When CDN sees this header, it bypasses the function routing and fetches from origin normally, preventing infinite loops. This is an extra safety measure in case BOT-xxx is still a match.

## Akamai Delivery Configuration

### Property Manager Rule

```json
{
  "name": "HTML-2-MD for bots",
  "criteria": [
    {
      "name": "matchVariable",
      "options": {
        "variableName": "PMUSER_BOT",
        "matchOperator": "IS_ONE_OF",
        "variableValues": ["BOT-69105154"]
      }
    },
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
        "headerName": "x-bvm-bypass-key",
        "matchOperator": "IS_NOT_ONE_OF",
        "values": ["production-secure-key-change-me"]
      }
    }
  ],
  "criteriaMustSatisfy": "all"
}
```

**Criteria Breakdown:**

1. **Bot Detection** (`PMUSER_BOT = "BOT-69105154"`)
   - Checks if Akamai Bot Manager detected a specific AI bot
   - Bot ID set via BVM rules in your property configuration
   - Example: `BOT-69105154` could represent GPTBot, ClaudeBot, etc.
   - You can create your own custom bot list and combine your own bots with known Akamai bots in 1 BOT-xxxx id.

2. **Path Match** (`/html`)
   - Only triggers for HTML content paths
   - Prevents function calls for assets (CSS, JS, images)
   - Customize to match your content paths

3. **No Bypass Key** (`x-bvm-bypass-key != "production-secure-key-change-me"`)
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
    "ttl": "5m",
    "prefreshable": true,
    "prefreshWindow": "80%"
  }
}
```

**Caching Details:**

- **TTL**: 5 minutes (adjust based on content freshness needs)
- **Prefresh**: Enabled at 80% (4 minutes)
  - Fresh copy fetched in background before TTL expires
  - Ensures zero cache misses for popular content
- **Cache Key**: Includes full URL (from `x-origin-url` header)
- **Separate Cache**: Bot requests cached separately from regular user requests

## Build & Run

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (1.78+)
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

**Note:** For local testing, the `BVM_BYPASS_KEY` is set to `"production-secure-key-change-me"` in `spin.toml`. Change this for production.

### Deploy to Akamai Functions

```bash
spin aka deploy
```

Your function will be deployed and you'll receive a URL like:

```
https://your-app-uuid.fwf.app
```

## Configuration

### Environment Variables

Set in `spin.toml`:

```toml
[component.html-to-md.environment]
BVM_BYPASS_KEY = "production-secure-key-change-me"
```

**BVM_BYPASS_KEY**: Secure key that CDN checks to bypass function routing

- Must match the value in your Akamai property configuration
- Used for loop prevention
- Change this to a secure, random value in production

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

**Errors:**

- `400` - Missing/invalid `x-origin-url` header, invalid Base64, non-HTTPS URL
- `422` - Remote returned non-2xx status, non-HTML content, response too large (>10 MiB), conversion failed
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
[html-2-md] WARN: BVM_BYPASS_KEY not set - requests may be blocked by BVM
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
- Non-HTTPS URL (400 error)

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

The `x-bvm-bypass-key` mechanism ensures the function doesn't create infinite routing loops:

1. Function adds `x-bvm-bypass-key: production-secure-key-change-me` to outbound requests
2. CDN checks this header value
3. If present and matches → bypass function routing, fetch from origin
4. If not present → normal BVM detection and routing

**Important:** Keep the bypass key secret and synchronized between:

- `spin.toml` environment variable
- Akamai property configuration criteria

### Size Limits

- Maximum response size: 10 MiB
- Prevents memory exhaustion in WebAssembly runtime
- Akamai Functions enforces similar limits

### Header Filtering

The function strips these headers from outbound requests:

- `x-origin-url` (component-specific)
- `host` (set to target host)
- Hop-by-hop headers: `connection`, `transfer-encoding`, `te`, `trailer`, `upgrade`
- Proxy headers: `proxy-authorization`, `proxy-authenticate`
- Compression: `accept-encoding`, `keep-alive`

## Performance

### Optimization

- **Edge Caching**: 5-minute TTL reduces function invocations
- **Prefresh**: Background refresh ensures cache hits
- **WebAssembly**: Near-native performance
- **Minimal Dependencies**: Fast cold starts

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

### Issue: Getting 422 error in loop

**Cause:** BVM bypass key mismatch

**Solution:**

1. Check `spin.toml` has `BVM_BYPASS_KEY = "production-secure-key-change-me"`
2. Verify Akamai property criteria checks for same value
3. Check function logs for actual key being sent

### Issue: Function not being called

**Cause:** Criteria not matching

**Solution:**

1. Verify `PMUSER_BOT` variable is set by BVM
2. Check path matches your content paths
3. Ensure request doesn't already have bypass key header

### Issue: Response too large error

**Cause:** HTML page exceeds 10 MiB

**Solution:**

- Implement pagination for large pages
- Use more specific selectors to extract only needed content
- Consider increasing `MAX_BODY_SIZE` constant (requires Akamai Functions tier check)

## Dependencies

- [spin-sdk](https://crates.io/crates/spin-sdk) v5.2.0 - Spin framework runtime
- [html-to-markdown-rs](https://crates.io/crates/html-to-markdown-rs) v3.11.0 - HTML to Markdown conversion
- [url](https://crates.io/crates/url) v2 - URL parsing and validation
- [base64](https://crates.io/crates/base64) v0.22 - Base64 encoding/decoding
- [anyhow](https://crates.io/crates/anyhow) v1 - Error handling

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
