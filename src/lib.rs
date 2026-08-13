// ============================================================================
// html-2-md — Spin HTTP component
// ============================================================================
//
// This is a WebAssembly (Wasm) component that runs on the Akamai Functions Spin runtime.
// My very first Rust code! Dumped a lot of learning resources into this file, so apologies for the wall of text.
// The code itself is only about 100 lines, but I wanted to explain every part in detail for Rust/Wasm/Spin newcomers like myself.
// Spin is a framework for building server-side Wasm applications — think of it
// like a lightweight, isolated web server where each request runs a Wasm module.
//
// What this component does:
//   1. Receives an HTTP request containing an "x-origin-url" header
//   2. Validates that the header value is a real https URL
//   3. Fetches the HTML at that URL (following redirects up to 10 times)
//   4. Validates the response is HTML and within the size limit
//   5. Converts the HTML to Markdown optimized for AI consumption
//   6. Returns the Markdown as the HTTP response body
//
// In Rust, a file called lib.rs defines a *library crate* (as opposed to main.rs
// which would be an executable). Spin looks for a library crate because it loads
// our code as a Wasm component, not a standalone binary.
// ============================================================================

// `use` statements bring external types into scope so we can refer to them by
// their short name instead of the full module path every time.
//
// Import the types we need from the Spin SDK's HTTP module.
// - send: makes an outbound HTTP request from within the Wasm component
// - IntoResponse: a trait that allows our return type to be converted into an HTTP response
// - Method: enum for HTTP methods (GET, POST, etc.)
// - Request / Response: represent inbound and outbound HTTP messages
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use html_to_markdown_rs::{convert, ConversionOptions, HeadingStyle};
use spin_sdk::http::{send, IntoResponse, Method, Request, Response};
use std::env;
use url::Url;

// The #[http_component] macro marks this Wasm module as a Spin HTTP component.
// Spin uses this to wire up the incoming HTTP trigger to our handler function.
use spin_sdk::http_component;

// Maximum response body size we'll accept from the remote server.
// Akamai Functions managed Spin service enforces a 10 MiB limit; we match that here
// to avoid running out of Wasm memory processing a huge page.
// 10 * 1024 * 1024 = 10,485,760 bytes
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

// Helper that builds a JSON error response. All error paths use this so the
// caller always gets a consistent `{"error": "..."}` body rather than plain text.
// `impl Into<String>` means we accept both &str and String without the caller
// having to call .to_string() every time.
fn error_json(status: u16, message: impl Into<String>) -> Response {
    let body = format!("{{\"error\":\"{}\"}}", message.into());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .build()
}

// The `async` keyword makes this an async function, required because we use `await`
// when making the outbound HTTP call. Spin supports async Wasm components.
//
// The function receives the incoming HTTP request as `req` and returns either
// an error (anyhow::Result) or something that can be turned into an HTTP response.
// `impl IntoResponse` means "any type that implements the IntoResponse trait" —
// in practice this will always be a `Response`.
#[http_component]
async fn handle_html_2_md(req: Request) -> anyhow::Result<impl IntoResponse> {

    // Try to read the "x-origin-url" header from the incoming request.
    // The header value is Base64 URL-encoded by the Akamai CDN to safely handle special characters,
    // query parameters, and Unicode in URLs.
    // req.header() returns Option<&HeaderValue>, and_then() chains another
    // Option-returning call to convert it to Option<&str>.
    // The `match` then handles both cases: header present (Some) or missing (None).
    let encoded_url = match req.header("x-origin-url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => {
            println!("[html-2-md] 400 missing x-origin-url header");
            return Ok(error_json(400, "Missing required header: x-origin-url"));
        }
    };

    // Decode the Base64 URL-encoded header value
    // Note: Akamai's base64_encode() may strip padding, so we need to add it back if missing
    let padded_url = match encoded_url.len() % 4 {
        0 => encoded_url.to_string(),
        n => format!("{}{}", encoded_url, "=".repeat(4 - n)),
    };

    let url = match URL_SAFE.decode(&padded_url) {
        Ok(decoded_bytes) => match String::from_utf8(decoded_bytes) {
            Ok(decoded_str) => {
                println!("[html-2-md] received request | base64: {} | decoded: {}", encoded_url, decoded_str);
                decoded_str
            }
            Err(_) => {
                println!("[html-2-md] 400 invalid UTF-8 in decoded URL");
                return Ok(error_json(400, "Invalid UTF-8 in decoded x-origin-url"));
            }
        },
        Err(e) => {
            println!("[html-2-md] 400 invalid Base64 encoding: {} | error: {:?}", encoded_url, e);
            return Ok(error_json(400, "Invalid Base64 encoding in x-origin-url header"));
        }
    };

    // Parse and validate the URL using the `url` crate — this checks the full structure,
    // not just the prefix. We only allow https:// for security — plain http sends
    // data unencrypted and is increasingly blocked by servers anyway.
    // The parsed URL also normalizes and properly encodes the URL for the outbound request.
    let parsed_url = match Url::parse(&url) {
        Ok(parsed) if parsed.scheme() == "https" => parsed,
        Ok(parsed) => {
            println!("[html-2-md] 400 invalid URL scheme: {} (must be https)", parsed.scheme());
            return Ok(error_json(400, "Invalid URL: x-origin-url must use https"));
        }
        Err(e) => {
            println!("[html-2-md] 400 invalid URL format: {} | error: {:?}", url, e);
            return Ok(error_json(400, "Invalid URL: x-origin-url is not a valid URL"));
        }
    };

    // Use the properly encoded URL string from the parsed URL object
    // This ensures special characters in query strings are correctly encoded for HTTP
    let current_url = parsed_url.as_str().to_string();

    // Follow redirects up to this many times before giving up.
    const MAX_REDIRECTS: usize = 10;
    // Note on timeouts: Akamai Functions enforces request timeouts at the runtime level (~30s default).
    // For local testing with `spin up`, Spin itself will terminate long-running requests.
    // Timeouts are applied by the Spin runtime, not at the HTTP client level.

    // Collect headers from the incoming request to forward to the outbound fetch.
    // We skip headers that are specific to this component, hop-by-hop headers that
    // must not be forwarded, and the host header (the outbound target has its own host).
    const SKIP_HEADERS: &[&str] = &[
        "x-origin-url", "host", "connection", "transfer-encoding",
        "te", "trailer", "upgrade", "proxy-authorization", "proxy-authenticate",
        "accept-encoding", "keep-alive",
    ];
    let forward_headers: Vec<(String, String)> = req
        .headers()
        .filter(|(name, _)| {
            let lower = name.to_lowercase();
            !SKIP_HEADERS.contains(&lower.as_str())
        })
        .filter_map(|(name, value)| value.as_str().map(|v| (name.to_string(), v.to_string())))
        .collect();

    // Read BVM bypass key from environment (fail-secure: warn if missing, don't fail)
    let bvm_bypass_key = match env::var("BVM_BYPASS_KEY") {
        Ok(key) if !key.is_empty() => Some(key),
        _ => {
            println!("[html-2-md] WARN: BVM_BYPASS_KEY not set - requests may be blocked by BVM");
            None
        }
    };

    // `current_url` was initialized earlier from the parsed URL and tracks the URL we're fetching.
    // It changes on each redirect. In Rust vars are immutable by default, so we declared it as
    // `mut` to allow updates inside the loop below.
    let mut current_url = current_url; // Make it mutable for redirect handling
    let mut redirects = 0usize; // same as `let mut redirects: usize = 0;`

    // `loop { ... break value; }` is a Rust idiom: the loop runs until we either
    // `break` with a value (success) or `return` early (error). The final `response`
    // variable is assigned whatever value we `break` with.
    let response: Response = loop {
        // Build and send the request for the current URL, forwarding the original
        // request headers (cookies, auth, etc.) and adding the tracking and BVM bypass headers.
        let outbound_req = match &bvm_bypass_key {
            Some(key) => forward_headers
                .iter()
                .fold(
                    Request::builder().method(Method::Get).uri(&current_url),
                    |builder, (name, value)| builder.header(name, value),
                )
                .header("x-aka-function", "html2md/1.0")
                .header("x-bvm-bypass-key", key)
                .build(),
            None => forward_headers
                .iter()
                .fold(
                    Request::builder().method(Method::Get).uri(&current_url),
                    |builder, (name, value)| builder.header(name, value),
                )
                .header("x-aka-function", "html2md/1.0")
                .build(),
        };

        // `.await` suspends until the response arrives. `send` returns a Result,
        // so we match on Ok (got a response) or Err (network failure).
        let resp: Response = match send(outbound_req).await {
            Ok(r) => r,
            Err(_) => return Ok(error_json(502, format!("Failed to fetch: {current_url}"))),
        };

        let status = resp.status();

        // Handle redirects (301, 302, 303, 307, 308 etc.).
        if (300..400).contains(status) {
            if redirects >= MAX_REDIRECTS {
                return Ok(error_json(502, format!("Too many redirects (max {MAX_REDIRECTS})")));
            }

            // Read the Location header — it tells us where to redirect to.
            // It can be an absolute URL (https://example.com/new) or a relative path (/new).
            let location = match resp.header("location").and_then(|v| v.as_str()) {
                Some(loc) => loc.to_string(),
                None => return Ok(error_json(502, format!("Redirect {status} received without a Location header"))),
            };

            // Resolve relative redirects against the current URL using the `url` crate.
            // e.g. current = "https://example.com/foo", location = "/bar"
            //   → new URL  = "https://example.com/bar"
            current_url = Url::parse(&current_url)
                .and_then(|base| base.join(&location))
                .map(|u| u.to_string())
                .unwrap_or(location); // Fall back to the raw Location value if parsing fails

            println!("[html-2-md] redirect {status} → {current_url}");
            redirects += 1;
            continue; // Go back to the top of the loop with the new URL
        }

        // Non-redirect, non-2xx status — the remote returned an error.
        if !matches!(status, 200..=299) {
            println!("[html-2-md] 422 remote error | url: {} | remote status: {}", current_url, status);
            return Ok(error_json(422, format!("Remote returned status {status}")));
        }

        // Success — exit the loop with this response.
        break resp;
    };

    // Check that the remote actually returned HTML. Servers sometimes return
    // PDFs, images, or other content types we can't convert. The content-type
    // header looks like "text/html; charset=utf-8" so we use contains() rather
    // than an exact match.
    let content_type = response
        .header("content-type")
        .and_then(|v| v.as_str())
        .unwrap_or(""); // If there's no content-type header, treat it as empty

    // Return 415, unsupported media type (not 422), so the EdgeWorker knows to forward the request to origin as-is.
    if !content_type.contains("text/html") {
        println!("[html-2-md] 415 non-html content-type: {content_type} {current_url}");
        return Ok(error_json(415, format!("Non-HTML content-type: {content_type}")));
    }

    // Check the response body isn't too large before we load it into memory.
    // Wasm has limited memory — Akamai Functions's managed service enforces a 10 MiB limit.
    let body_bytes = response.body();
    if body_bytes.len() > MAX_BODY_SIZE {
        return Ok(error_json(422, format!("Response too large ({} bytes, max {MAX_BODY_SIZE})", body_bytes.len())));
    }

    // Extract the response body as a UTF-8 string.
    // String::from_utf8_lossy converts bytes to a string, replacing any invalid
    // UTF-8 sequences with the replacement character (â). into_owned() converts
    // the result from a borrowed Cow<str> into an owned String.
    let html = if body_bytes.is_empty() {
        return Ok(error_json(422, "Empty response from remote"));
    } else {
        String::from_utf8_lossy(body_bytes).into_owned()
    };
    let html_kib = body_bytes.len() / 1024;

    // Convert the HTML string to Markdown using the html-to-markdown-rs crate.
    // The second argument accepts an Option<ConversionOptions> for customization;
    // all convert options can be set using the builder pattern. Here we specify that we want ATX-style and skip images.
    // https://docs.rs/html-to-markdown-rs/latest/html_to_markdown_rs/options/conversion/struct.ConversionOptions.html
    // Claude advised these options for better AI text processing, but you can experiment with different settings to see what works best for your use case.
    let options = ConversionOptions::builder()
        .heading_style(HeadingStyle::Atx) // Use # for headings
        .skip_images(true)                // Images are useless for AI text processing
        .strip_tags(vec![                 // Vec Rust macro to create a Vec (growable array. Used to remove boilerplate HTML elements irrelevant to content
            "nav".to_string(),            // API expects a Vec<String>, so we convert string literals (&str) to String objects with .to_string()
            "footer".to_string(),
            "aside".to_string(),
            "script".to_string(),
            "style".to_string(),
        ])
        .extract_metadata(false)
        .autolinks(true)                  // Cleaner auto-link URL representation
        .wrap(false)                      // No hard line wrapping — cleaner paragraphs for AI
        .default_title(true)              // Always include a title even if the page omits one
        .build();

    // Provide our convert options to the convert function. It returns a Result, so we match on Ok/Err.
    // On success, result.content holds the converted Markdown string.
    let markdown = match convert(&html, Some(options)) {
        Ok(result) => result.content,
        Err(_) => return Ok(error_json(422, "Failed to convert HTML to Markdown")),
    };
    let md_kib = markdown.as_deref().unwrap_or("").len() / 1024;
    println!("[html-2-md] html: {} KiB  →  md: {} KiB  |  url: {}", html_kib, md_kib, current_url);

    // Everything went well — return the Markdown with a 200 OK.
    // The content-type is "text/markdown" so callers know what they received.
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/markdown; charset=utf-8")
        .body(markdown)
        .build())
}
