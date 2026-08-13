# Test 1: Valid request with new header
GET {{base_url}}/
x-origin-url: https://www.medemblik.nl
HTTP 200
[Asserts]
header "content-type" contains "text/markdown"

# Test 2: Missing x-origin-url header
GET {{base_url}}/
HTTP 400
[Asserts]
jsonpath "$.error" contains "x-origin-url"

# Test 3: Invalid x-origin-url (non-HTTPS)
GET {{base_url}}/
x-origin-url: http://example.com
HTTP 400
[Asserts]
jsonpath "$.error" contains "https"
