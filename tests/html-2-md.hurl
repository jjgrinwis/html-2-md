# Test 1: Valid request with Base64-encoded URL
# Must target /html, not the origin root — the root serves application/json,
# which the component relays unchanged instead of converting (see Test 4).
GET {{base_url}}/
x-origin-url: aHR0cHM6Ly9haS1ib3QuZ3JlYXQtZGVtby5jb20vaHRtbA==
HTTP 200
[Asserts]
header "content-type" contains "text/markdown"

# Test 2: Missing x-origin-url header
GET {{base_url}}/
HTTP 400
[Asserts]
jsonpath "$.error" contains "x-origin-url"

# Test 3: Invalid Base64 encoding
GET {{base_url}}/
x-origin-url: not-valid-base64!!!
HTTP 400
[Asserts]
jsonpath "$.error" contains "Base64"

# Test 4: Non-HTML content-type is relayed unchanged, not converted.
# The origin root serves application/json, so the component must pass the
# original status, content-type and body straight back to the caller.
GET {{base_url}}/
x-origin-url: aHR0cHM6Ly9haS1ib3QuZ3JlYXQtZGVtby5jb20v
HTTP 200
[Asserts]
header "content-type" contains "application/json"
jsonpath "$.message" exists
