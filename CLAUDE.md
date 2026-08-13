# CLAUDE.md

## What this is

A [Spin](https://spinframework.dev/) WebAssembly HTTP component written in Rust. It accepts an incoming HTTP request with an `x-origin-url` header, fetches that URL, converts the HTML to Markdown using `html-to-markdown-rs`, and returns the Markdown. Intended for use by AI bots detected by Akamai Bot Manager (BVM).

## Build & run

```bash
# Build the Wasm component and run Spin
spin build && spin up

# Call it (in another terminal) - URL must be Base64 URL-encoded
BASE64_URL=$(echo -n "https://example.com" | base64)
curl -H "x-origin-url: $BASE64_URL" http://localhost:3000/
```

`spin build` automatically compiles with `--target wasm32-wasip2 --release`.

## Request headers

- **`x-origin-url`** (required): Base64 URL-encoded full URL to fetch
  - Set by Akamai delivery configuration when BVM detects an AI bot
  - Must be Base64 URL-safe encoded (e.g., `aHR0cHM6Ly93d3cuZXhhbXBsZS5jb20vcGFnZQ==` for `https://www.example.com/page`)
  - Decoded URL must be valid HTTPS
  - Function fetches this URL, which typically points back to the same delivery configuration

## Outbound request headers

The function adds this header to outbound requests:

- **`x-aka-function: html2md/1.0`** — Identifies function-initiated requests and prevents routing loops

## Loop prevention

To prevent infinite loops (CDN → Function → BVM → Function), the function adds `x-aka-function: html2md/1.0` to all outbound requests:

- Akamai delivery configuration checks for this header
- When present, CDN bypasses function routing and fetches from origin directly
- No environment variables needed — loop prevention is built into the function behavior

## Security: Outbound host restrictions

The function is restricted to only fetch from authorized domains via `allowed_outbound_hosts` in `spin.toml`:

```toml
allowed_outbound_hosts = ["https://ai-bot.great-demo.com"]
```

**What this prevents:**
- **Open proxy abuse** — prevents the function from being used to fetch arbitrary URLs
- **SSRF attacks** — blocks attempts to probe internal networks or unauthorized services
- **Resource abuse** — limits outbound requests to your authorized domain only

**How it works:**
- Enforced at the WebAssembly runtime level by Spin
- If the decoded `x-origin-url` points to a non-allowed host, the request fails with `502 Bad Gateway`
- Error message: `"Failed to fetch: https://unauthorized-domain.com/"`
- Spin logs show: `ERROR spin_runtime_factors: Outbound network destination not allowed`

**To allow multiple domains or subdomains:**
```toml
allowed_outbound_hosts = [
    "https://ai-bot.great-demo.com",
    "https://*.great-demo.com"  # Allows all subdomains
]
```

This is defense-in-depth: even if someone bypasses CDN configuration, the Wasm function physically cannot fetch from unauthorized hosts.

## Request timeouts

Request timeouts are enforced by the Spin runtime:

- **Akamai Functions**: Default ~30 second timeout at runtime level
- **Local testing** (`spin up`): Spin enforces its own request limits

Timeouts are applied automatically by the hosting environment, not at the HTTP client level.

## Deploy to Akamai Functions

This component is designed to run on [Akamai Functions](https://www.akamai.com/products/serverless-computing) using the Spin runtime.

### Prerequisites

Install the Akamai plugin for Spin:

```bash
spin plugins install aka
```

### Deployment steps

1. **Authenticate** with your Akamai account:

   ```bash
   spin aka login
   ```

2. **Deploy** the component (takes a few seconds):
   ```bash
   spin aka deploy
   ```

Your component will be built and deployed to Akamai Functions. The command will output the public HTTPS URL where it's accessible.


### Calling the deployed function

Once deployed, call it with a Base64 URL-encoded URL:

```bash
BASE64_URL=$(echo -n "https://example.com" | base64)
curl -H "x-origin-url: $BASE64_URL" https://your-akamai-function-url/
```

### Akamai CDN Configuration

In your Akamai Property Manager configuration, encode the original request URL using Base64:

```
Match: Variable AK_FIREWALL_DETECTED_RULES matches "BOT-*"
Then:
  - Set Variable PMUSER_ORIGIN_URL_BASE64 = base64_encode(concat("https://", builtin.AK_HOST, builtin.AK_PATH, builtin.AK_QUERY))
  - Forward to Akamai Function
  - Modify Outgoing Request Header: 
      Action: Add
      Header Name: x-origin-url
      Header Value: {{user.PMUSER_ORIGIN_URL_BASE64}}
```

#### About Bot IDs (BOT-*)

The `BOT-*` pattern matches bot detection rules from Akamai Bot Manager (BVM). Each custom bot list you create gets a unique bot ID. Custom bot lists can include:

- **Your own bot definitions** based on request headers, cookies, client-lists, user-agents, etc.
- **Akamai-defined bots** (pre-classified bots from Akamai's threat intelligence)
- **Combination of both** — a single custom bot group can mix your custom rules with Akamai's bot categories

For example, `AK_FIREWALL_DETECTED_RULES` might contain values like:
- `BOT-12345` (your custom bot list combining header-based detection + Akamai's AI crawler category)
- `BOT-67890` (another custom list for search engine bots)

The function will:
1. Decode the Base64 URL
2. Fetch the content from that URL (going back through your CDN)
3. Convert HTML to Markdown
4. Return optimized content for caching

## Architecture

- **`src/lib.rs`** — the entire application. A single async `#[http_component]` handler function.
- **`spin.toml`** — Spin manifest: declares the HTTP trigger route, points to the compiled `.wasm`, and sets `allowed_outbound_hosts` (must include any hosts the component fetches from).
- **`Cargo.toml`** — key dependencies: `spin-sdk`, `html-to-markdown-rs`, `url`, `anyhow`.

The request flow is:

1. Read `x-origin-url` header → Base64 URL-decode it
2. Validate decoded URL is a well-formed HTTPS URL using the `url` crate
3. Fetch the page via `spin_sdk::http::send` — follow redirects (up to 10) with relative URL resolution
4. Add outbound header: `x-aka-function: html2md/1.0` (for loop prevention)
5. Validate response is HTML (check `content-type` header) and within 10 MiB size limit
6. Convert HTML → Markdown via `html_to_markdown_rs::convert` with AI-optimized `ConversionOptions`
7. Return `200 text/markdown` with Markdown body, or JSON error object on failure

## Error responses

All error responses return JSON: `{"error": "error message"}`.

| Status | Condition                                                                                                                           |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| 400    | Missing or invalid `x-origin-url` header; URL scheme must be HTTPS                                                                  |
| 422    | Remote returned non-2xx status; non-HTML content-type; response body exceeds 10 MiB; empty response body; or HTML conversion failed |
| 502    | Network failure fetching URL; too many redirects (max 10); or missing Location header on redirect response                          |

Success: `200 text/markdown; charset=utf-8` with Markdown body

## Conversion options

`ConversionOptions` in `src/lib.rs` are tuned for AI consumption:

- **Heading style**: ATX (`#`, `##`, etc.) for consistent Markdown
- **Images**: Skipped (not useful for AI text processing)
- **Boilerplate removal**: `nav`, `footer`, `aside`, `script`, `style` tags stripped
- **Metadata extraction**: Disabled (not currently used)
- **Autolinks**: Enabled (cleaner link representation)
- **Line wrapping**: Disabled (preserves paragraph structure)
- **Default title**: Always included, even if page omits one

See the [html-to-markdown-rs ConversionOptions documentation](https://docs.rs/html-to-markdown-rs/latest/html_to_markdown_rs/options/conversion/struct.ConversionOptions.html) and [ConversionOptionsBuilder](https://docs.rs/html-to-markdown-rs/latest/html_to_markdown_rs/options/conversion/struct.ConversionOptionsBuilder.html) for all available options.
