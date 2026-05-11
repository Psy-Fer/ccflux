-- Total tokens per user, last 30 days
SELECT
    user_email,
    SUM(input_tokens + output_tokens)   AS total_tokens,
    SUM(input_tokens)                   AS input_tokens,
    SUM(output_tokens)                  AS output_tokens,
    SUM(cache_read_tokens)              AS cache_read_tokens,
    SUM(cache_write_tokens)             AS cache_write_tokens,
    COUNT(DISTINCT session_id)          AS sessions,
    COUNT(*)                            AS turns,
    MAX(timestamp_utc)                  AS last_active
FROM usage_events
WHERE timestamp_utc >= datetime('now', '-30 days')
GROUP BY user_email
ORDER BY total_tokens DESC;
