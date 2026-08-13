# Performance test - Direct to Akamai CDN
# This tests the full flow: CDN -> Function -> Cache
# URL: https://ai-bot.great-demo.com/html

GET https://ai-bot.great-demo.com/html
HTTP 200
[Asserts]
duration < 5000
header "content-type" exists
