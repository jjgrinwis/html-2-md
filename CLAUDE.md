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

In your Akamai Property Manager configuration, bot detection and the function trigger are split across two rules because `builtin.AK_FIREWALL_TRIGGERED_RULES` is only reliably populated on the edge server that terminates the client request — it does not survive being read again at a parent/peer/child tier. To make the detected bot ID available wherever the caching/forwarding decision is actually evaluated, it's propagated via an explicit request header instead:

```
Rule: Detect Bot (criteria: requestType IS CLIENT_REQ)
Then:
  - Set Variable PMUSER_BOT = {{builtin.AK_FIREWALL_TRIGGERED_RULES}}
  - Modify Outgoing Request Header:
      Action: Add
      Header Name: x-detected-bot
      Header Value: {{user.PMUSER_BOT}}

Rule: HTML-2-MD for bots (criteria: path matches "/html" AND header x-aka-function DOES_NOT_EXIST)
  Child rule matches if EITHER:
    - Variable PMUSER_BOT IS_ONE_OF [<bot-id-1>, <bot-id-2>]
    - Header x-detected-bot IS_ONE_OF [<bot-id-1>, <bot-id-2>]
  Then:
    - Set Variable PMUSER_ORIGIN_URL = base64_url_encode(concat("https://", builtin.AK_HOST, builtin.AK_URL))
    - Modify Outgoing Request Header:
        Action: Add
        Header Name: x-origin-url
        Header Value: {{user.PMUSER_ORIGIN_URL}}
    - Origin: forward to the Akamai Function (e.g. a `<function-id>.fwf.app` hostname)
    - Caching: MAX_AGE, ttl 2m
```

The last part of the "HTML-2-MD for bots" criteria (`x-aka-function DOES_NOT_EXIST`) is important; otherwise you can get into a loop, where the function ends up fetching its own `text/markdown` output. Note this loop no longer surfaces as an error status: because `text/markdown` isn't HTML, the function relays it back unchanged, so the response still looks correct while silently costing an extra edge round trip per request. Watch for `[html-2-md] passthrough non-html content-type: text/markdown` in the logs — that line means the loop guard isn't working.

#### About Bot IDs (BOT-*)

The `BOT-*` pattern (e.g. `BOT-12345`) identifies bot detection rules from Akamai Bot Manager (BVM). Each custom bot list you create gets a unique bot ID. Custom bot lists can include:

- **Your own bot definitions** based on request headers, cookies, client-lists, user-agents, etc.
- **Akamai-defined bots** (pre-classified bots from Akamai's threat intelligence)
- **Combination of both** — a single custom bot group can mix your custom rules with Akamai's bot categories

Match against a specific list of known bot IDs rather than a `BOT-*` wildcard — the exact IDs for AI bots still need to be confirmed, so the current config lists them explicitly and checks both the `PMUSER_BOT` variable and the `x-detected-bot` header (belt-and-suspenders while the multi-tier propagation is validated).

The function will:
1. Decode the Base64 URL
2. Fetch the content from that URL (going back through your CDN)
3. Convert HTML to Markdown
4. Return optimized content for caching

## Architecture

- **`src/lib.rs`** — the entire application. A single async `#[http_component]` handler function.
- **`spin.toml`** — Spin manifest: declares the HTTP trigger route, points to the compiled `.wasm`, and sets `allowed_outbound_hosts` (must include any hosts the component fetches from).
- **`Cargo.toml`** — key dependencies: `spin-sdk`, `html-to-markdown-rs`, `url`, `anyhow`, `futures`.

The handler uses Spin's **streaming** (Input/Output Params) form — `async fn handle(req: Request, resp_out: ResponseOutparam)` with no return value — rather than the simpler `-> Result<impl IntoResponse>` form. This is required to relay non-HTML bodies without buffering them. Consequences: every error path calls `error_json(resp_out, ...).await` then `return` instead of returning a `Response`, and `resp_out` may be set exactly once (the compiler enforces this, since it is taken by value).

The request flow is:

1. Read `x-origin-url` header → Base64 URL-decode it
2. Validate decoded URL is a well-formed HTTPS URL using the `url` crate
3. Fetch the page via `spin_sdk::http::send` — follow redirects (up to 10) with relative URL resolution
4. Add outbound header: `x-aka-function: html2md/1.0` (for loop prevention)
5. Check `content-type` — if it isn't `text/html`, **stream** the remote response back unchanged (status, content-type, body) and stop here; no size limit applies on this path
6. Otherwise accumulate the body chunk by chunk, rejecting it once it exceeds the 10 MiB conversion limit
7. Convert HTML → Markdown via `html_to_markdown_rs::convert` with AI-optimized `ConversionOptions`
8. Return `200 text/markdown` with Markdown body, or JSON error object on failure

## Error responses

All error responses return JSON: `{"error": "error message"}`.

| Status | Condition                                                                                                                           |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| 400    | Missing or invalid `x-origin-url` header; invalid Base64; invalid UTF-8 in decoded URL; URL scheme must be HTTPS                    |
| 422    | Remote returned non-2xx status; HTML exceeds the 10 MiB conversion limit; empty response body; or HTML conversion failed             |
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
