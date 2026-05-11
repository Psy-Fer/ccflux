-- Token consumption by model across the org.
SELECT
    model,
    COUNT(DISTINCT user_email)          AS unique_users,
    COUNT(*)                            AS turns,
    SUM(input_tokens)                   AS total_input,
    SUM(output_tokens)                  AS total_output,
    SUM(cache_read_tokens)              AS total_cache_read,
    SUM(cache_write_tokens)             AS total_cache_write,
    ROUND(
        100.0 * SUM(cache_read_tokens)
        / NULLIF(SUM(input_tokens + cache_read_tokens + cache_write_tokens), 0),
        1
    )                                   AS cache_hit_pct
FROM usage_events
GROUP BY model
ORDER BY total_output DESC;
