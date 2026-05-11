-- Token usage per 5-hour reset window (seat pressure indicator).
-- Each CC seat resets its token limit every 5 hours from session start.
SELECT
    user_email,
    session_id,
    session_start_utc,
    CAST(
        (JULIANDAY(timestamp_utc) - JULIANDAY(session_start_utc)) * 24 / 5
        AS INTEGER
    )                                   AS five_hour_window,
    SUM(input_tokens + output_tokens)   AS tokens_in_window,
    SUM(output_tokens)                  AS output_tokens,
    COUNT(*)                            AS turns
FROM usage_events
WHERE session_start_utc IS NOT NULL
GROUP BY user_email, session_id, five_hour_window
ORDER BY tokens_in_window DESC;
