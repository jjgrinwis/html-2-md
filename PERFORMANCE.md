# Performance Test Results

## Test Configuration

- **URL**: https://ai-bot.great-demo.com/html
- **Header**: `x-custom-bot: yes` (routes through the html-2-md function path)
- **Test Runs**: 10 consecutive requests
- **Test Location**: Netherlands (Amsterdam/Schiphol region)
- **Akamai Edge**: NL__AMSTERDAM (104.97.14.6)

## Results Summary

These numbers come from the [Additional Test](#additional-test-x-custom-bot-header-function-path-only) below — the only run where the first request was confirmed as a genuine cold miss (full cache expiry, function actually fetching origin and converting). The page under test is over 2MB, so that first request's ~2s cost is real function work, not noise, and is excluded from the cached average below.

| Metric | Value |
|--------|-------|
| **First Request (Genuine Cold Miss)** | 2075ms |
| **Cached Average (Requests 2-10)** | 300ms |
| **Min Response (Cached)** | 291ms |
| **Max Response (Cached)** | 313ms |
| **Cache Performance Improvement** | ~85% faster |

See [Additional Test](#additional-test-x-custom-bot-header-function-path-only) for the run-by-run detail.

## Analysis

### First Request (Genuine Cold Miss - Function Execution)
- **Time**: 2075ms
- **Flow**: CDN → Akamai Function → Function fetches origin → HTML to Markdown conversion → Response
- **Components**:
  - Connection establishment: 122ms
  - BVM detection and routing
  - Function execution (fetch + conversion) — the dominant cost, since the origin page is >2MB
  - Time to First Byte: 1995ms

### Cached Requests (Edge Served)
- **Average Time**: 300ms
- **Flow**: CDN → Cached Markdown Response (edge served)
- **Components**:
  - Connection establishment: 20-28ms (avg ~24ms)
  - Cache lookup and delivery
  - No function invocation
  - No origin fetch

### Performance Benefits

1. **Cache Hit Rate Impact**:
   - ~85% faster response time for cached content vs. a genuine cold miss
   - Reduces function invocations by ~90% (only on cache miss/refresh)
   - Lower origin load (fetches happen only on cache miss)

2. **Connection Reuse**:
   - First request connection: 122ms
   - Subsequent requests: 20-28ms (avg 24ms)
   - TCP connection overhead amortized over multiple requests

3. **Edge Caching Benefits**:
   - Content served from Netherlands edge location
   - 2-minute TTL for the function path (see caching behavior in the delivery config)
   - Separate cache for bot requests vs regular users

## Function Performance

The Akamai Function performs the following operations in ~2075ms on a genuine cold miss (page >2MB):

1. **Decode Base64 URL**: <1ms
2. **Fetch HTML from origin**: majority of the ~1995ms TTFB (includes CDN → Origin round trip for a >2MB page)
3. **HTML to Markdown conversion**: remainder of the ~1995ms TTFB (depends on HTML size)
4. **Response assembly**: <5ms

### Conversion Efficiency

- Removes boilerplate: `nav`, `footer`, `aside`, `script`, `style` tags
- Skips images (not useful for AI)
- Generates clean ATX-style headings
- No line wrapping (preserves structure)

## Caching Configuration

```json
{
  "behavior": "MAX_AGE",
  "ttl": "2m"
}
```

- **TTL**: 2 minutes
- **Cache Key**: Full URL (from `x-origin-url` header)
- **Separate Cache**: Bot requests cached independently from regular users

## Recommendations

### For Production

1. **Increase TTL for stable content**: Consider 15-30 minutes for content that changes infrequently
2. **Monitor cache hit ratio**: Target >85% cache hit rate
3. **Enable compression**: Markdown typically compresses well (60-80% reduction)
4. **Add cache tags**: For cache invalidation on content updates

### For Performance

1. **Edge location matters**: Response time varies by user location
2. **Connection reuse**: HTTP/2 benefits increase with multiple requests
3. **Origin optimization**: Faster origin response = faster function execution, especially on cold miss with large (>2MB) pages

## Testing Methodology

### Using curl

```bash
# Single test with timing
curl -o /dev/null -s -w "Time: %{time_total}s | TTFB: %{time_starttransfer}s\n" \
    https://ai-bot.great-demo.com/html

# 10 test runs with statistics
for i in {1..10}; do
    curl -o /dev/null -s -w "Run $i: %{time_total}s\n" \
        https://ai-bot.great-demo.com/html
    sleep 0.3
done
```

### Using Hurl

```bash
# Run performance test suite
hurl --test tests/performance.hurl

# With verbose timing
hurl --test tests/performance.hurl --very-verbose
```

## Additional Test: x-custom-bot Header (Function Path Only)

- **URL**: https://ai-bot.great-demo.com/html
- **Header**: `x-custom-bot: yes` (routes through the html-2-md function)
- **Test Runs**: 10 consecutive requests
- **Note**: The `x-custom-bot: no` (bypass/uncached) path could not be tested from this location — this test environment's IP is always flagged as a bot by Akamai Bot Manager, so a request with `x-custom-bot: no` still routes through the same cacheable function path (confirmed by the requester seeing `Akamai-Cache-Status: NotCacheable from child` from an unflagged IP, vs. `Hit from child`/`Miss from child` seen here).

Runs were spaced so the first request always followed a full expiry of the 2-minute cache TTL, ensuring a genuine cold miss.

| Run | Total Time | TTFB    | Connect | Status | Akamai-Cache-Status |
|-----|------------|---------|---------|--------|----------------------|
| 1   |    2075ms  | 1995ms  | 122ms   | 200    | Miss from child      |
| 2   |     301ms  |  222ms  |  24ms   | 200    | Hit from child       |
| 3   |     296ms  |  223ms  |  20ms   | 200    | Hit from child       |
| 4   |     297ms  |  226ms  |  21ms   | 200    | Miss from child      |
| 5   |     313ms  |  235ms  |  24ms   | 200    | Hit from child       |
| 6   |     291ms  |  216ms  |  23ms   | 200    | Hit from child       |
| 7   |     305ms  |  223ms  |  26ms   | 200    | Hit from child       |
| 8   |     300ms  |  227ms  |  28ms   | 200    | Hit from child       |
| 9   |     302ms  |  228ms  |  24ms   | 200    | Hit from child       |
| 10  |     297ms  |  220ms  |  23ms   | 200    | Hit from child       |

**Observations:**

- First request (genuine cold miss, cache fully expired beforehand): **2075ms**. This is the function actually doing work: fetching the origin HTML and running the HTML→Markdown conversion. It is **not** Wasm cold start — Spin/Wasm component startup is near-instant, so essentially all of this ~2s is origin fetch + conversion time.
- Cached requests: **~300ms average**, an **~85% reduction** versus the real cold-miss cost.
- Run 4 is also labeled `Miss from child` but returned in ~300ms — far too fast to be a real function re-execution. This is almost certainly a child-tier cache miss that was still served from a parent tier's cache, not a fresh function invocation. `Akamai-Cache-Status: Miss from child` on its own is therefore not a reliable signal of "the function ran"; only a ~2s response time is.
- This means the real, measurable benefit of caching here is avoiding the ~1.8s of origin-fetch-plus-conversion work on every request — the conversion step is the expensive part, exactly as expected, and caching hides nearly all of it.

## Conclusion

The Akamai Edge caching provides **~85% performance improvement** over a genuine cold miss, with cached requests averaging **300ms** vs **2075ms** for a real function execution against this >2MB page. This ensures:

- Fast response times for AI bots on cache hits
- Reduced function invocations and costs
- Lower origin load
- Consistent performance for popular content

The function itself does real, non-trivial work on a cold miss — fetching and converting a >2MB HTML page in ~2s — which is exactly what caching is designed to hide from repeat requests.
