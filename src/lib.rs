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
//   4. Streams the response straight back to the caller if it isn't HTML (a PDF, JSON, an image)
//   5. Otherwise buffers the HTML, up to a 10 MiB limit, and converts it to Markdown for AI consumption
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
// - Method: enum for HTTP methods (GET, POST, etc.)
// - Request / IncomingResponse / OutgoingResponse: inbound and outbound HTTP messages
// - Fields: a set of HTTP headers
// - ResponseOutparam: the slot a streaming handler writes its response into
// - SinkExt / TryStreamExt: extension traits from the `futures` crate that give us
//   `.send()` on a body sink and `.try_next()` on a body stream. Streaming is why
//   they're here: we pump the remote response through this component chunk by chunk
//   instead of loading it all into memory.
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use futures::{SinkExt, TryStreamExt};
use html_to_markdown_rs::{convert, ConversionOptions, HeadingStyle};
use spin_sdk::http::{
    send, Fields, IncomingResponse, Method, OutgoingResponse, Request, ResponseOutparam,
};
use url::Url;

// The #[http_component] macro marks this Wasm module as a Spin HTTP component.
// Spin uses this to wire up the incoming HTTP trigger to our handler function.
use spin_sdk::http_component;

// Maximum HTML size we'll buffer in memory before converting.
// This limit exists *only* for the conversion path: html-to-markdown-rs needs the
// whole document in memory at once, so a huge page could exhaust Wasm memory.
// Responses we merely relay (see the passthrough path below) are streamed and
// therefore have no size limit at all — a 500 MB PDF costs us one chunk of memory.
// 10 * 1024 * 1024 = 10,485,760 bytes
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

// Helper that writes a complete (non-streaming) response into the outparam.
//
// Streaming components don't `return` a response — they're handed a
// `ResponseOutparam` and must set it exactly once. That's why these helpers take
// `resp_out` by value: it can only be used once, and the compiler enforces it.
async fn respond(resp_out: ResponseOutparam, status: u16, content_type: &str, body: Vec<u8>) {
    let headers = Fields::new();
    let _ = headers.append(&"content-type".to_string(), &content_type.as_bytes().to_vec());

    let response = OutgoingResponse::new(headers);
    let _ = response.set_status_code(status);

    if let Err(e) = resp_out.set_with_body(response, body).await {
        eprintln!("[html-2-md] failed to write response body: {e}");
    }
}

// Helper that reads a single header value from an incoming response as a String.
//
// Unlike the buffered `Response` type, an `IncomingResponse` exposes headers as raw
// bytes, and any header name can legitimately appear more than once — so `get`
// returns a Vec. We want the first value, decoded as text.
fn header_value(response: &IncomingResponse, name: &str) -> Option<String> {
    response
        .headers()
        .get(&name.to_string())
        .into_iter()
        .next()
        .map(|v| String::from_utf8_lossy(&v).into_owned())
}

// Helper that builds a JSON error response. All error paths use this so the
// caller always gets a consistent `{"error": "..."}` body rather than plain text.
// `impl Into<String>` means we accept both &str and String without the caller
// having to call .to_string() every time.
async fn error_json(resp_out: ResponseOutparam, status: u16, message: impl Into<String>) {
    let body = format!("{{\"error\":\"{}\"}}", message.into()).into_bytes();
    respond(resp_out, status, "application/json", body).await;
}

// The `async` keyword makes this an async function, required because we use `await`
// when making the outbound HTTP call. Spin supports async Wasm components.
//
// This handler uses Spin's *streaming* form: two parameters and no return value.
// The first is the incoming request; the second is a slot we write the response
// into. Spin picks this form automatically when the function takes two arguments.
//
// We need it because a response we merely relay (a PDF, an image, JSON) should
// flow straight through this component rather than being buffered in Wasm memory
// first. The buffered `-> Result<impl IntoResponse>` form cannot do that: it has
// to have the whole body in hand before it can return anything.
#[http_component]
async fn handle_html_2_md(req: Request, resp_out: ResponseOutparam) {

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
            error_json(resp_out, 400, "Missing required header: x-origin-url").await;
            return;
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
                error_json(resp_out, 400, "Invalid UTF-8 in decoded x-origin-url").await;
                return;
            }
        },
        Err(e) => {
            println!("[html-2-md] 400 invalid Base64 encoding: {} | error: {:?}", encoded_url, e);
            error_json(resp_out, 400, "Invalid Base64 encoding in x-origin-url header").await;
            return;
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
            error_json(resp_out, 400, "Invalid URL: x-origin-url must use https").await;
            return;
        }
        Err(e) => {
            println!("[html-2-md] 400 invalid URL format: {} | error: {:?}", url, e);
            error_json(resp_out, 400, "Invalid URL: x-origin-url is not a valid URL").await;
            return;
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

    // `current_url` was initialized earlier from the parsed URL and tracks the URL we're fetching.
    // It changes on each redirect. In Rust vars are immutable by default, so we declared it as
    // `mut` to allow updates inside the loop below.
    let mut current_url = current_url; // Make it mutable for redirect handling
    let mut redirects = 0usize; // same as `let mut redirects: usize = 0;`

    // `loop { ... break value; }` is a Rust idiom: the loop runs until we either
    // `break` with a value (success) or `return` early (error). The final `response`
    // variable is assigned whatever value we `break` with.
    let response: IncomingResponse = loop {
        // Build and send the request for the current URL, forwarding the original
        // request headers (cookies, auth, etc.) and adding the x-aka-function header.
        // The x-aka-function header identifies requests from this function and is used
        // by the CDN to prevent routing loops (CDN checks for this header and bypasses
        // function routing when present).
        let outbound_req = forward_headers
            .iter()
            .fold(
                Request::builder().method(Method::Get).uri(&current_url),
                |builder, (name, value)| builder.header(name, value),
            )
            .header("x-aka-function", "html2md/1.0")
            .build();

        // `.await` suspends until the response arrives. `send` returns a Result,
        // so we match on Ok (got a response) or Err (network failure).
        //
        // We ask for an `IncomingResponse` rather than a `Response`. The difference
        // matters: a `Response` reads the entire body into memory before `send`
        // returns, while an `IncomingResponse` gives us the status and headers as
        // soon as they arrive and lets us read the body separately — as a stream,
        // or into a buffer, whichever the content-type turns out to need.
        let resp: IncomingResponse = match send(outbound_req).await {
            Ok(r) => r,
            Err(_) => {
                error_json(resp_out, 502, format!("Failed to fetch: {current_url}")).await;
                return;
            }
        };

        let status = resp.status();

        // Handle redirects (301, 302, 303, 307, 308 etc.).
        if (300..400).contains(&status) {
            if redirects >= MAX_REDIRECTS {
                error_json(resp_out, 502, format!("Too many redirects (max {MAX_REDIRECTS})")).await;
                return;
            }

            // Read the Location header — it tells us where to redirect to.
            // It can be an absolute URL (https://example.com/new) or a relative path (/new).
            // Header values on an IncomingResponse are raw bytes and a name can appear
            // more than once, so `get` hands back a Vec — we take the first entry.
            let location = match header_value(&resp, "location") {
                Some(loc) => loc,
                None => {
                    error_json(resp_out, 502, format!("Redirect {status} received without a Location header")).await;
                    return;
                }
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
            error_json(resp_out, 422, format!("Remote returned status {status}")).await;
            return;
        }

        // Success — exit the loop with this response.
        break resp;
    };

    let status = response.status();

    // Check that the remote actually returned HTML. Servers sometimes return
    // PDFs, images, or other content types we can't convert. The content-type
    // header looks like "text/html; charset=utf-8" so we use contains() rather
    // than an exact match.
    let content_type = header_value(&response, "content-type").unwrap_or_default();

    // We can only convert HTML. Anything else (JSON, PDFs, images, plain text) is
    // relayed back to the caller unchanged, so the CDN can serve the original content
    // without needing failover rules to recover from an error status here.
    //
    // This is the streaming path. We never hold the whole body: each chunk read from
    // the remote is written straight to the client, so memory use stays flat no matter
    // how big the file is, and the client starts receiving bytes immediately.
    if !content_type.contains("text/html") {
        println!("[html-2-md] passthrough non-html content-type: {content_type} | {current_url}");

        let headers = Fields::new();
        if !content_type.is_empty() {
            let _ = headers.append(&"content-type".to_string(), &content_type.as_bytes().to_vec());
        }
        let outgoing = OutgoingResponse::new(headers);
        let _ = outgoing.set_status_code(status);

        // Take the body sink *before* handing the response to Spin — once we call
        // `set`, the response value is gone. Setting it early is what makes this
        // streaming: the client gets headers now and body chunks as they arrive.
        let mut sink = outgoing.take_body();
        let stream = response.take_body_stream();
        resp_out.set(outgoing);

        // `pin!` fixes the stream in place. Streams have to be pinned before they
        // can be polled, because they may hold references into themselves.
        let mut stream = std::pin::pin!(stream);
        let mut relayed = 0usize;

        loop {
            match stream.try_next().await {
                Ok(Some(chunk)) => {
                    relayed += chunk.len();
                    if let Err(e) = sink.send(chunk).await {
                        // The response is already in flight, so we can't switch to an
                        // error status — all we can do is stop and log it.
                        eprintln!("[html-2-md] passthrough write failed after {relayed} bytes: {e}");
                        return;
                    }
                }
                Ok(None) => break, // end of body
                Err(e) => {
                    eprintln!("[html-2-md] passthrough read failed after {relayed} bytes: {e}");
                    return;
                }
            }
        }

        // Flush and close so the client knows the response is complete rather than
        // truncated. Without this the connection can be left hanging.
        let _ = sink.flush().await;
        let _ = sink.close().await;
        println!("[html-2-md] passthrough complete | {relayed} bytes | {current_url}");
        return;
    }

    // The conversion path. html-to-markdown-rs needs the entire document in memory,
    // so here we do have to buffer — but we accumulate chunk by chunk and check the
    // limit as we go. That way an oversized page is rejected after ~10 MiB instead of
    // being fully allocated first and measured afterwards.
    let mut body_bytes: Vec<u8> = Vec::new();
    let stream = response.take_body_stream();
    let mut stream = std::pin::pin!(stream);

    loop {
        match stream.try_next().await {
            Ok(Some(chunk)) => {
                if body_bytes.len() + chunk.len() > MAX_BODY_SIZE {
                    println!("[html-2-md] 422 response too large | url: {current_url}");
                    error_json(resp_out, 422, format!("Response too large (max {MAX_BODY_SIZE} bytes)")).await;
                    return;
                }
                body_bytes.extend(chunk);
            }
            Ok(None) => break,
            Err(e) => {
                error_json(resp_out, 502, format!("Failed to read body from {current_url}: {e}")).await;
                return;
            }
        }
    }

    if body_bytes.is_empty() {
        error_json(resp_out, 422, "Empty response from remote").await;
        return;
    }

    // Extract the response body as a UTF-8 string.
    // String::from_utf8_lossy converts bytes to a string, replacing any invalid
    // UTF-8 sequences with the replacement character. into_owned() converts
    // the result from a borrowed Cow<str> into an owned String.
    let html = String::from_utf8_lossy(&body_bytes).into_owned();
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
        Ok(result) => result.content.unwrap_or_default(),
        Err(_) => {
            error_json(resp_out, 422, "Failed to convert HTML to Markdown").await;
            return;
        }
    };
    let md_kib = markdown.len() / 1024;
    println!("[html-2-md] html: {} KiB  →  md: {} KiB  |  url: {}", html_kib, md_kib, current_url);

    // Everything went well — return the Markdown with a 200 OK.
    // The content-type is "text/markdown" so callers know what they received.
    // The Markdown is already fully in memory, so there's nothing to stream here.
    respond(resp_out, 200, "text/markdown; charset=utf-8", markdown.into_bytes()).await;
}
