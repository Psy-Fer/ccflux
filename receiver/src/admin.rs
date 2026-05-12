use std::collections::HashMap;

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};

use subtle::ConstantTimeEq;

use crate::{db, AppState};

const COOKIE_NAME: &str = "ccflux_admin";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin", get(handle_dashboard))
        .route("/admin/", get(handle_dashboard))
        .route(
            "/admin/login",
            get(handle_login_get).post(handle_login_post),
        )
        .route("/admin/device-keys/revoke", post(handle_revoke_key))
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(admin_token) = &state.admin_token else {
        return false;
    };
    if let Some(v) = headers.get("authorization") {
        if let Some(t) = v.to_str().ok().and_then(|s| s.strip_prefix("Bearer ")) {
            if ct_eq(t, admin_token) {
                return true;
            }
        }
    }
    if let Some(v) = headers.get("cookie") {
        if let Ok(s) = v.to_str() {
            let prefix = format!("{COOKIE_NAME}=");
            for c in s.split(';') {
                if let Some(v) = c.trim().strip_prefix(&prefix) {
                    if ct_eq(v, admin_token) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn disabled_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Html("<p>Admin dashboard is disabled. Set the ADMIN_TOKEN environment variable to enable it.</p>"),
    )
        .into_response()
}

pub async fn handle_login_get(State(state): State<AppState>) -> Response {
    if state.admin_token.is_none() {
        return disabled_response();
    }
    Html(login_page(None)).into_response()
}

pub async fn handle_login_post(
    State(state): State<AppState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(admin_token) = &state.admin_token else {
        return disabled_response();
    };
    match form.get("token") {
        Some(t) if ct_eq(t, admin_token) => {
            let secure = if state.cookie_secure { "; Secure" } else { "" };
            axum::http::Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, "/admin/")
                .header(
                    header::SET_COOKIE,
                    format!("{COOKIE_NAME}={admin_token}; HttpOnly; Path=/admin; SameSite=Strict{secure}"),
                )
                .body(Body::empty())
                .unwrap()
                .into_response()
        }
        _ => Html(login_page(Some("Invalid token"))).into_response(),
    }
}

pub async fn handle_revoke_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if state.admin_token.is_none() {
        return disabled_response();
    }
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }
    if let Some(key) = form.get("pubkey") {
        let _ = db::admin_revoke_key(&state.pool, key).await;
    }
    Redirect::to("/admin/").into_response()
}

pub async fn handle_dashboard(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if state.admin_token.is_none() {
        return disabled_response();
    }
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }

    let data = tokio::try_join!(
        db::admin_org_summary(&state.pool),
        db::admin_user_stats(&state.pool),
        db::admin_model_stats(&state.pool),
        db::admin_daily_stats(&state.pool),
        db::admin_recent_events(&state.pool),
        db::admin_device_keys(&state.pool),
    );

    let (summary, users, models, daily, recent, devices) = match data {
        Ok(d) => d,
        Err(e) => {
            eprintln!("admin db error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now_dt = chrono::Utc::now();
    let now_iso = now_dt.to_rfc3339();
    let now_display = now_dt.format("%Y-%m-%d %H:%M UTC").to_string();
    let cache_hit_pct = {
        let total = summary.total_input + summary.total_cache_read + summary.total_cache_write;
        if total > 0 {
            format!(
                "{:.1}%",
                100.0 * summary.total_cache_read as f64 / total as f64
            )
        } else {
            "—".to_string()
        }
    };

    let cards = format!(
        r#"<div class="cards">
  {card1}{card2}{card3}{card4}{card5}{card6}
</div>"#,
        card1 = card("Users", &summary.total_users.to_string()),
        card2 = card("Sessions", &fmt_num(summary.total_sessions)),
        card3 = card("Turns", &fmt_num(summary.total_turns)),
        card4 = card("Input", &fmt_num(summary.total_input)),
        card5 = card("Output", &fmt_num(summary.total_output)),
        card6 = card("Cache hit", &cache_hit_pct),
    );

    let chart_svg = svg_line_chart(&daily);
    let chart = format!(
        r#"<div class="panel">
  <div class="panel-head">Daily billed tokens — last 30 days</div>
  <div class="chart-wrap">{chart_svg}</div>
</div>"#
    );

    let user_bar_rows: Vec<(&str, i64, i64)> = users
        .iter()
        .map(|u| (u.email.as_str(), u.input_tokens + u.output_tokens, 0i64))
        .collect();
    let user_bar_svg = svg_hbar_chart(&user_bar_rows);
    let user_bar = format!(
        r#"<div class="panel">
  <div class="panel-head">Billed tokens by user — last 30 days</div>
  <div class="chart-wrap">{user_bar_svg}</div>
</div>"#
    );

    let model_bar_rows: Vec<(&str, i64, i64)> = models
        .iter()
        .map(|m| (m.model.as_str(), m.total_input + m.total_output, 0i64))
        .collect();
    let model_bar_svg = svg_hbar_chart(&model_bar_rows);
    let model_bar = format!(
        r#"<div class="panel">
  <div class="panel-head">Billed tokens by model — all time</div>
  <div class="chart-wrap">{model_bar_svg}</div>
</div>"#
    );

    let user_rows: String = if users.is_empty() {
        r#"<tr><td colspan="8" class="empty">No data in the last 30 days</td></tr>"#.to_string()
    } else {
        users
            .iter()
            .map(|u| {
                format!(
                    r#"<tr>
  <td>{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="mono sm">{}</td>
</tr>"#,
                    esc(&u.email),
                    fmt_num(u.input_tokens),
                    fmt_num(u.output_tokens),
                    fmt_num(u.cache_read_tokens),
                    fmt_num(u.cache_write_tokens),
                    u.sessions,
                    u.turns,
                    ts(&u.last_active),
                )
            })
            .collect()
    };

    let user_table = format!(
        r#"<div class="panel">
  <div class="panel-head">Usage by user — last 30 days</div>
  <table>
    <thead><tr>
      <th>Email</th><th>Input</th><th>Output</th>
      <th>Cache reads</th><th>Cache writes</th>
      <th>Sessions</th><th>Turns</th><th>Last active</th>
    </tr></thead>
    <tbody>{user_rows}</tbody>
  </table>
</div>"#
    );

    let model_rows: String = if models.is_empty() {
        r#"<tr><td colspan="8" class="empty">No data</td></tr>"#.to_string()
    } else {
        models
            .iter()
            .map(|m| {
                format!(
                    r#"<tr>
  <td class="mono">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td>
</tr>"#,
                    esc(&m.model),
                    m.unique_users,
                    m.turns,
                    fmt_num(m.total_input),
                    fmt_num(m.total_output),
                    fmt_num(m.total_cache_read),
                    fmt_num(m.total_cache_write),
                    fmt_pct(m.cache_hit_pct),
                )
            })
            .collect()
    };

    let model_table = format!(
        r#"<div class="panel">
  <div class="panel-head">Model breakdown</div>
  <table>
    <thead><tr>
      <th>Model</th><th>Users</th><th>Turns</th>
      <th>Input</th><th>Output</th><th>Cache reads</th><th>Cache writes</th><th>Cache hit %</th>
    </tr></thead>
    <tbody>{model_rows}</tbody>
  </table>
</div>"#
    );

    let key_rows: String = if devices.is_empty() {
        r#"<tr><td colspan="6" class="empty">No device keys registered</td></tr>"#.to_string()
    } else {
        devices
            .iter()
            .map(|k| {
                let status = if k.revoked {
                    r#"<span class="badge rev">Revoked</span>"#
                } else {
                    r#"<span class="badge ok">Active</span>"#
                };
                let revoke_btn = if !k.revoked {
                    format!(
                        r#"<form class="inline" method="post" action="/admin/device-keys/revoke"
                                onsubmit="return confirm('Revoke this device key?')">
              <input type="hidden" name="pubkey" value="{}">
              <button type="submit" class="btn-revoke">Revoke</button>
            </form>"#,
                        esc(&k.public_key)
                    )
                } else {
                    String::new()
                };
                format!(
                    r#"<tr>
  <td>{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td>{status} {revoke_btn}</td>
</tr>"#,
                    esc(&k.email),
                    esc(&k.device_id),
                    esc(&short_key(&k.public_key)),
                    ts(&k.registered_at),
                    ts(&k.last_seen_at),
                )
            })
            .collect()
    };

    let key_table = format!(
        r#"<div class="panel">
  <div class="panel-head">Device keys</div>
  <table>
    <thead><tr>
      <th>Email</th><th>Device</th><th>Key (short)</th>
      <th>Registered</th><th>Last seen</th><th>Status</th>
    </tr></thead>
    <tbody>{key_rows}</tbody>
  </table>
</div>"#
    );

    let event_rows: String = if recent.is_empty() {
        r#"<tr><td colspan="9" class="empty">No events yet</td></tr>"#.to_string()
    } else {
        recent
            .iter()
            .map(|e| {
                format!(
                    r#"<tr>
  <td class="mono sm">{}</td>
  <td>{}</td>
  <td class="mono sm">{}</td>
  <td class="num">{}</td>
  <td class="mono sm">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
</tr>"#,
                    ts(&e.received_at),
                    esc(&e.user_email),
                    esc(&e.session_id[..8]),
                    e.turn_index,
                    esc(&e.model),
                    fmt_num(e.input_tokens),
                    fmt_num(e.output_tokens),
                    fmt_num(e.cache_read_tokens),
                    fmt_num(e.cache_write_tokens),
                )
            })
            .collect()
    };

    let event_table = format!(
        r#"<div class="panel">
  <div class="panel-head">Recent events — last 50</div>
  <table>
    <thead><tr>
      <th>Received</th><th>User</th><th>Session</th><th>Turn</th>
      <th>Model</th><th>Input</th><th>Output</th><th>Cache reads</th><th>Cache writes</th>
    </tr></thead>
    <tbody>{event_rows}</tbody>
  </table>
</div>"#
    );

    let body = format!(
        r#"<div class="topbar">
  <h1>ccflux admin</h1>
  <span class="sub">Updated <span data-utc="{now_iso}">{now_display}</span></span>
</div>
<div class="wrap">
  {cards}
  {chart}
  {user_bar}
  {model_bar}
  {user_table}
  {model_table}
  {key_table}
  {event_table}
</div>"#
    );

    Html(page_shell("Dashboard", &body)).into_response()
}

// ── HTML helpers ─────────────────────────────────────────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_num(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_pct(f: f64) -> String {
    format!("{f:.1}%")
}

fn short_key(k: &str) -> String {
    if k.len() <= 20 {
        return k.to_string();
    }
    format!("{}…{}", &k[..8], &k[k.len() - 8..])
}

/// Normalise any UTC timestamp string to ISO 8601 with a Z suffix.
fn to_utc_iso(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    if s.contains('T') {
        if s.ends_with('Z') || s.contains('+') {
            s.to_string()
        } else {
            format!("{s}Z")
        }
    } else {
        // SQLite CURRENT_TIMESTAMP: "YYYY-MM-DD HH:MM:SS"
        format!("{}Z", s.replace(' ', "T"))
    }
}

/// Wraps a UTC timestamp in a <span data-utc="…"> so the inline JS can
/// rewrite it to the browser's local timezone on page load.
fn ts(s: &str) -> String {
    if s.is_empty() || s == "—" {
        return "—".to_string();
    }
    let iso = to_utc_iso(s);
    let display = s.get(..16).unwrap_or(s);
    format!(r#"<span data-utc="{}">{}</span>"#, esc(&iso), esc(display))
}

fn card(label: &str, value: &str) -> String {
    format!(
        r#"<div class="card"><div class="lbl">{label}</div><div class="val">{value}</div></div>"#
    )
}

fn svg_line_chart(stats: &[db::AdminDailyStat]) -> String {
    // Single-quoted SVG attributes throughout to avoid "# terminating r#"…"# raw strings.
    let (w, h, pl, pr, pt, pb) = (760i64, 160i64, 60i64, 16i64, 16i64, 36i64);
    let (pw, ph) = (w - pl - pr, h - pt - pb);

    let n = stats.len();
    if n == 0 {
        return format!(
            "<svg width='{w}' height='{h}' xmlns='http://www.w3.org/2000/svg'>\
             <text x='{}' y='{}' text-anchor='middle' fill='#aaa' font-size='13'>No data yet</text>\
             </svg>",
            w / 2,
            h / 2
        );
    }

    let max_val = stats
        .iter()
        .map(|s| s.input_tokens + s.output_tokens)
        .max()
        .unwrap_or(1)
        .max(1);

    let x_at = |i: usize| -> i64 {
        if n <= 1 {
            pl + pw / 2
        } else {
            pl + (i as i64 * pw) / (n - 1) as i64
        }
    };
    let y_at = |v: i64| -> i64 { pt + ph - (v.max(0) * ph / max_val).min(ph) };

    let line_pts: String = stats
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{},{}", x_at(i), y_at(s.input_tokens + s.output_tokens)))
        .collect::<Vec<_>>()
        .join(" ");

    let area_pts = format!(
        "{line_pts} {},{} {},{}",
        x_at(n - 1),
        pt + ph,
        x_at(0),
        pt + ph
    );

    let mut svg = format!(
        "<svg width='{w}' height='{h}' xmlns='http://www.w3.org/2000/svg' style='max-width:100%;display:block'>"
    );

    // Y grid and labels
    for v in [0i64, max_val / 2, max_val] {
        let y = y_at(v);
        let x2 = pl + pw;
        let tx = pl - 4;
        let ty = y + 4;
        let lbl = fmt_num(v);
        svg.push_str(&format!(
            "<line x1='{pl}' y1='{y}' x2='{x2}' y2='{y}' stroke='#f0f0f0' stroke-width='1'/>\
             <text x='{tx}' y='{ty}' text-anchor='end' fill='#aaa' font-size='10'>{lbl}</text>"
        ));
    }

    // Y axis rule
    let y_bottom = pt + ph;
    svg.push_str(&format!(
        "<line x1='{pl}' y1='{pt}' x2='{pl}' y2='{y_bottom}' stroke='#e0e0e0' stroke-width='1'/>"
    ));

    // Filled area + line
    svg.push_str(&format!(
        "<polygon points='{area_pts}' fill='#4f8ef7' fill-opacity='0.12'/>\
         <polyline points='{line_pts}' fill='none' stroke='#4f8ef7' stroke-width='2' \
           stroke-linejoin='round' stroke-linecap='round'/>"
    ));

    // X axis date labels
    let label_y = pt + ph + 14;
    for (i, s) in stats
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || (*i + 1) % 7 == 0 || *i == n - 1)
    {
        let x = x_at(i);
        let lbl = s.day.get(5..10).unwrap_or(&s.day);
        svg.push_str(&format!(
            "<text x='{x}' y='{label_y}' text-anchor='middle' fill='#aaa' font-size='10'>{lbl}</text>"
        ));
    }

    svg.push_str("</svg>");
    svg
}

/// Horizontal bar chart — one bar per entry, labelled on the left.
/// `rows`: (label, value, total) — bar fills value/total of the plot width.
fn svg_hbar_chart(rows: &[(&str, i64, i64)]) -> String {
    if rows.is_empty() {
        return "<p style='color:#aaa;padding:1rem'>No data</p>".to_string();
    }
    let bar_h = 26i64;
    let gap = 6i64;
    let label_w = 200i64;
    let value_w = 60i64;
    let plot_w = 760i64 - label_w - value_w - 16;
    let h = rows.len() as i64 * (bar_h + gap) + gap;

    let max_val = rows.iter().map(|r| r.1).max().unwrap_or(1).max(1);

    let mut svg = format!(
        "<svg width='760' height='{h}' xmlns='http://www.w3.org/2000/svg' style='max-width:100%;display:block'>"
    );
    for (i, (label, val, _total)) in rows.iter().enumerate() {
        let y = gap + i as i64 * (bar_h + gap);
        let bar_len = (val * plot_w / max_val).max(if *val > 0 { 2 } else { 0 });
        let tx = label_w + plot_w + 8;
        let ty = y + bar_h / 2 + 4;
        let lbl_y = y + bar_h / 2 + 4;
        svg.push_str(&format!(
            "<text x='{label_w_m4}' y='{lbl_y}' text-anchor='end' font-size='12' fill='#1a1a2e' \
               font-family='SFMono-Regular,Consolas,monospace'>{label_esc}</text>\
             <rect x='{label_w}' y='{y}' width='{plot_w}' height='{bar_h}' rx='3' fill='#f4f6f8'/>\
             <rect x='{label_w}' y='{y}' width='{bar_len}' height='{bar_h}' rx='3' fill='#4f8ef7'/>\
             <text x='{tx}' y='{ty}' font-size='11' fill='#5a6676' \
               font-family='SFMono-Regular,Consolas,monospace'>{val_fmt}</text>",
            label_w_m4 = label_w - 8,
            label_esc = esc(label),
            val_fmt = fmt_num(*val),
        ));
    }
    svg.push_str("</svg>");
    svg
}

fn page_shell(title: &str, content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>{title} — ccflux</title>
  <style>
    *{{box-sizing:border-box;margin:0;padding:0}}
    body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:#f4f6f8;color:#1a1a2e;font-size:14px}}
    .topbar{{background:#1a1a2e;color:rgba(255,255,255,.9);padding:.75rem 1.5rem;display:flex;align-items:center}}
    .topbar h1{{font-size:1rem;font-weight:700;letter-spacing:.03em}}
    .topbar .sub{{font-size:.8rem;opacity:.5;margin-left:auto}}
    .wrap{{max-width:1200px;margin:0 auto;padding:1.5rem}}
    .cards{{display:flex;flex-wrap:wrap;gap:.75rem;margin-bottom:1.25rem}}
    .card{{background:white;border-radius:8px;padding:1rem 1.25rem;box-shadow:0 1px 3px rgba(0,0,0,.08);flex:1;min-width:130px}}
    .card .lbl{{font-size:.72rem;text-transform:uppercase;letter-spacing:.06em;color:#8894a4;margin-bottom:.3rem}}
    .card .val{{font-size:1.55rem;font-weight:700;color:#1a1a2e;line-height:1}}
    .panel{{background:white;border-radius:8px;box-shadow:0 1px 3px rgba(0,0,0,.08);margin-bottom:1.25rem;overflow:hidden}}
    .panel-head{{padding:.65rem 1.1rem;border-bottom:1px solid #eef0f3;font-size:.78rem;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:#5a6676}}
    .chart-wrap{{padding:1rem 1.1rem .75rem;overflow-x:auto}}
    table{{width:100%;border-collapse:collapse}}
    th{{padding:.45rem 1rem;background:#f8f9fb;border-bottom:1px solid #eef0f3;font-size:.72rem;font-weight:700;text-transform:uppercase;letter-spacing:.06em;color:#8894a4;text-align:left;white-space:nowrap}}
    td{{padding:.55rem 1rem;border-bottom:1px solid #f4f6f8;vertical-align:middle}}
    tr:last-child td{{border-bottom:none}}
    tbody tr:hover td{{background:#f8f9fb}}
    td.num{{text-align:right;font-variant-numeric:tabular-nums}}
    th:not(:first-child){{text-align:right}}
    .mono{{font-family:"SFMono-Regular",Consolas,monospace}}
    .sm{{font-size:.8rem;color:#5a6676}}
    .badge{{display:inline-block;padding:.2em .5em;border-radius:4px;font-size:.7rem;font-weight:600;text-transform:uppercase;letter-spacing:.04em}}
    .ok{{background:#dcfce7;color:#16a34a}}
    .rev{{background:#fee2e2;color:#dc2626}}
    .empty{{padding:2rem;text-align:center;color:#8894a4;font-size:.85rem}}
    form.inline{{display:inline}}
    .btn-revoke{{background:none;border:1px solid #dc2626;color:#dc2626;padding:.2rem .5rem;border-radius:4px;font-size:.72rem;cursor:pointer}}
    .btn-revoke:hover{{background:#dc2626;color:white}}
  </style>
</head>
<body>
{content}
<script>
document.querySelectorAll('[data-utc]').forEach(function(el){{
  var d=new Date(el.getAttribute('data-utc'));
  if(!isNaN(d.getTime()))el.textContent=d.toLocaleString(undefined,{{dateStyle:'short',timeStyle:'short'}});
}});
</script>
</body>
</html>"#
    )
}

fn login_page(error: Option<&str>) -> String {
    let err = error
        .map(|e| format!(r#"<p class="err">{}</p>"#, esc(e)))
        .unwrap_or_default();
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>ccflux admin login</title>
  <style>
    *{{box-sizing:border-box;margin:0;padding:0}}
    body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;background:#f4f6f8;display:flex;align-items:center;justify-content:center;height:100vh}}
    .box{{background:white;padding:2rem;border-radius:12px;box-shadow:0 4px 16px rgba(0,0,0,.1);width:320px}}
    h1{{font-size:1.1rem;font-weight:700;color:#1a1a2e;margin-bottom:1.5rem}}
    label{{display:block;font-size:.82rem;color:#555;margin-bottom:.3rem}}
    input[type=password]{{width:100%;padding:.6rem .75rem;border:1px solid #d0d7de;border-radius:6px;font-size:.95rem}}
    input[type=password]:focus{{outline:none;border-color:#4f8ef7;box-shadow:0 0 0 3px rgba(79,142,247,.15)}}
    button{{margin-top:1rem;width:100%;padding:.65rem;background:#1a1a2e;color:white;border:none;border-radius:6px;font-size:.95rem;cursor:pointer}}
    button:hover{{background:#2d2d54}}
    .err{{color:#dc2626;font-size:.82rem;margin:.5rem 0}}
  </style>
</head>
<body>
  <div class="box">
    <h1>ccflux admin</h1>
    {err}
    <form method="post">
      <label>Admin token</label>
      <input type="password" name="token" autofocus required>
      <button type="submit">Sign in</button>
    </form>
  </div>
</body>
</html>"#
    )
}
