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
        .route("/admin/users/provision", post(handle_provision_user))
        .route("/admin/users/revoke", post(handle_revoke_user))
        .route("/admin/users/reissue", post(handle_reissue_token))
        .route("/admin/logout", get(handle_logout))
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
    let Some(admin_token) = &state.admin_token else {
        return disabled_response();
    };
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }
    let csrf_ok = form
        .get("csrf_token")
        .map(|t| ct_eq(t, admin_token))
        .unwrap_or(false);
    if !csrf_ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(key) = form.get("pubkey") {
        let _ = db::admin_revoke_key(&state.pool, key).await;
    }
    Redirect::to("/admin/").into_response()
}

pub async fn handle_logout(State(state): State<AppState>) -> Response {
    let secure = if state.cookie_secure { "; Secure" } else { "" };
    axum::http::Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/admin/login")
        .header(
            header::SET_COOKIE,
            format!("{COOKIE_NAME}=; HttpOnly; Path=/admin; SameSite=Strict; Max-Age=0{secure}"),
        )
        .body(Body::empty())
        .unwrap()
        .into_response()
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
        db::fetch_events_for_windows(&state.pool),
        db::admin_list_users(&state.pool),
    );

    let (summary, users, models, daily, recent, devices, raw_events, provisioned_users) = match data
    {
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
        r#"<div class="panel" id="p-daily">
  <div class="panel-head" onclick="togglePanel('p-daily')">Daily billed tokens — last 30 days <span class="chv">&#9660;</span></div>
  <div class="chart-wrap">{chart_svg}</div>
</div>"#
    );

    let user_bar_rows: Vec<(&str, i64, i64)> = users
        .iter()
        .map(|u| (u.email.as_str(), u.input_tokens + u.output_tokens, 0i64))
        .collect();
    let user_bar_svg = svg_hbar_chart(&user_bar_rows);
    let user_bar = format!(
        r#"<div class="panel" id="p-user-bar">
  <div class="panel-head" onclick="togglePanel('p-user-bar')">Billed tokens by user — last 30 days <span class="chv">&#9660;</span></div>
  <div class="chart-wrap">{user_bar_svg}</div>
</div>"#
    );

    let model_bar_rows: Vec<(&str, i64, i64)> = models
        .iter()
        .map(|m| (m.model.as_str(), m.total_input + m.total_output, 0i64))
        .collect();
    let model_bar_svg = svg_hbar_chart(&model_bar_rows);
    let model_bar = format!(
        r#"<div class="panel" id="p-model-bar">
  <div class="panel-head" onclick="togglePanel('p-model-bar')">Billed tokens by model — all time <span class="chv">&#9660;</span></div>
  <div class="chart-wrap">{model_bar_svg}</div>
</div>"#
    );

    let tier_snapshot = state.tier_cache.lock().await.clone();

    let user_rows: String = if users.is_empty() {
        r#"<tr><td colspan="9" class="empty">No data in the last 30 days</td></tr>"#.to_string()
    } else {
        users
            .iter()
            .map(|u| {
                let tier = tier_snapshot
                    .get(&u.email)
                    .cloned()
                    .unwrap_or_else(crate::tiers::TierClassification::unknown);
                let tier_cell = tier_badge(&tier);
                format!(
                    r#"<tr>
  <td>{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="mono sm">{}</td>
  <td>{tier_cell}</td>
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
        r#"<div class="panel" id="p-user-tbl">
  <div class="panel-head" onclick="togglePanel('p-user-tbl')">Usage by user — last 30 days <span class="chv">&#9660;</span></div>
  <table>
    <thead><tr>
      <th>Email</th><th>Input</th><th>Output</th>
      <th>Cache reads</th><th>Cache writes</th>
      <th>Sessions</th><th>Turns</th><th>Last active</th><th>Tier</th>
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
        r#"<div class="panel" id="p-model-tbl">
  <div class="panel-head" onclick="togglePanel('p-model-tbl')">Model breakdown <span class="chv">&#9660;</span></div>
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
                    let csrf = state.admin_token.as_deref().unwrap_or("");
                    format!(
                        r#"<form class="inline" method="post" action="/admin/device-keys/revoke"
                                onsubmit="return confirm('Revoke this device key?')">
              <input type="hidden" name="pubkey" value="{}">
              <input type="hidden" name="csrf_token" value="{}">
              <button type="submit" class="btn-revoke">Revoke</button>
            </form>"#,
                        esc(&k.public_key),
                        esc(csrf),
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
        r#"<div class="panel" id="p-keys">
  <div class="panel-head" onclick="togglePanel('p-keys')">Device keys <span class="chv">&#9660;</span></div>
  <div class="tbl-filter-wrap"><input class="tbl-filter" type="search" placeholder="Filter by email or device…" oninput="filterTable(this,'keys-body')"></div>
  <div class="tbl-scroll"><table>
    <thead><tr>
      <th>Email</th><th>Device</th><th>Key (short)</th>
      <th>Registered</th><th>Last seen</th><th>Status</th>
    </tr></thead>
    <tbody id="keys-body">{key_rows}</tbody>
  </table></div>
</div>"#
    );

    let event_rows: String = if recent.is_empty() {
        r#"<tr><td colspan="10" class="empty">No events yet</td></tr>"#.to_string()
    } else {
        recent
            .iter()
            .map(|e| {
                format!(
                    r#"<tr>
  <td class="mono sm">{}</td>
  <td>{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td class="num">{}</td>
  <td class="mono sm">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
</tr>"#,
                    ts(&e.received_at),
                    esc(&e.user_email),
                    esc(&e.device_id),
                    esc(e.session_id.get(..8).unwrap_or(&e.session_id)),
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
        r#"<div class="panel" id="p-events">
  <div class="panel-head" onclick="togglePanel('p-events')">Recent events — last 50 <span class="chv">&#9660;</span></div>
  <table>
    <thead><tr>
      <th>Received</th><th>User</th><th>Device</th><th>Session</th><th>Turn</th>
      <th>Model</th><th>Input</th><th>Output</th><th>Cache reads</th><th>Cache writes</th>
    </tr></thead>
    <tbody>{event_rows}</tbody>
  </table>
</div>"#
    );

    // ── User provisioning panel ──────────────────────────────────────────────

    let csrf = state.admin_token.as_deref().unwrap_or("");

    let prov_form = format!(
        r#"<form class="prov-form" method="post" action="/admin/users/provision">
  <input type="hidden" name="csrf_token" value="{csrf_esc}">
  <div class="prov-row">
    <input type="email" name="email" placeholder="user@example.org" required>
    <input type="text" name="division" placeholder="Division (optional)">
    <span class="prov-lbl">Days valid</span>
    <input type="number" name="expires_days" value="365" min="1" max="3650">
    <button type="submit" class="btn-issue">Add user</button>
  </div>
</form>"#,
        csrf_esc = esc(csrf),
    );

    let user_provision_rows: String = if provisioned_users.is_empty() {
        r#"<tr><td colspan="7" class="empty">No users provisioned yet</td></tr>"#.to_string()
    } else {
        provisioned_users
            .iter()
            .map(|u| {
                let (badge_cls, badge_lbl) = if u.revoked {
                    ("rev", "Revoked")
                } else if u.is_expired {
                    ("exp", "Expired")
                } else {
                    ("ok", "Active")
                };
                let status_html = format!(r#"<span class="badge {badge_cls}">{badge_lbl}</span>"#);
                let revoke_btn = if !u.revoked && !u.is_expired {
                    format!(
                        r#"<form class="inline" method="post" action="/admin/users/revoke"
                onsubmit="return confirm('Revoke this token?')">
          <input type="hidden" name="token" value="{}">
          <input type="hidden" name="csrf_token" value="{}">
          <button type="submit" class="btn-revoke">Revoke</button>
        </form>"#,
                        esc(&u.token),
                        esc(csrf),
                    )
                } else {
                    String::new()
                };
                let reissue_btn = if !u.revoked && !u.is_expired {
                    format!(
                        r#"<form class="inline" method="post" action="/admin/users/reissue"
                onsubmit="return confirm('Revoke current token and issue a new one?')">
          <input type="hidden" name="token" value="{}">
          <input type="hidden" name="csrf_token" value="{}">
          <button type="submit" class="btn-reissue">Reissue</button>
        </form>"#,
                        esc(&u.token),
                        esc(csrf),
                    )
                } else {
                    String::new()
                };
                let last = if u.last_active.is_empty() {
                    "—".to_string()
                } else {
                    ts(&u.last_active)
                };
                format!(
                    r#"<tr>
  <td>{}</td>
  <td class="sm">{}</td>
  <td>{status_html}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{last}</td>
  <td>{revoke_btn} {reissue_btn}</td>
</tr>"#,
                    esc(&u.email),
                    esc(&u.division),
                    ts(&u.created_at),
                    ts(&u.expires_at),
                )
            })
            .collect()
    };

    let users_panel = format!(
        r#"<div class="panel" id="p-provision">
  <div class="panel-head" onclick="togglePanel('p-provision')">User provisioning <span class="chv">&#9660;</span></div>
  {prov_form}
  <div class="tbl-filter-wrap"><input class="tbl-filter" type="search" placeholder="Filter by email or division…" oninput="filterTable(this,'prov-body')"></div>
  <div class="tbl-scroll"><table>
    <thead><tr>
      <th>Email</th><th>Division</th><th>Status</th>
      <th>Created</th><th>Expires</th><th>Last active</th><th>Actions</th>
    </tr></thead>
    <tbody id="prov-body">{user_provision_rows}</tbody>
  </table></div>
</div>"#
    );

    // ── 5-hour billing windows (ccusage algorithm) ────────────────────────────

    let windows = crate::billing::compute_billing_windows(raw_events);

    // Bar chart: peak billed tokens across all windows per user (last 30 days).
    // Pairs are (label, peak, avg) — avg rendered as a lighter secondary bar.
    let mut peak_by_user: HashMap<&str, (i64, i64, usize)> = HashMap::new(); // email → (peak, sum, count)
    for w in &windows {
        let e = peak_by_user
            .entry(w.user_email.as_str())
            .or_insert((0, 0, 0));
        let b = w.billed_tokens();
        if b > e.0 {
            e.0 = b;
        }
        e.1 += b;
        e.2 += 1;
    }
    let mut peak_rows: Vec<(&str, i64, i64)> = peak_by_user
        .iter()
        .map(|(email, (peak, sum, count))| {
            let avg = if *count > 0 { sum / *count as i64 } else { 0 };
            (*email, *peak, avg)
        })
        .collect();
    peak_rows.sort_by_key(|b| std::cmp::Reverse(b.1));

    let win_bar_svg = svg_hbar_chart(&peak_rows);
    let win_bar_note: String = peak_rows
        .iter()
        .map(|(email, peak, avg)| {
            let active_note = windows
                .iter()
                .find(|w| w.user_email == *email && w.is_active)
                .map(|w| {
                    format!(
                        " &nbsp;<span class='badge ok'>active</span> {} billed this window — resets {}",
                        fmt_num(w.billed_tokens()),
                        ts(&w.end.to_rfc3339()),
                    )
                })
                .unwrap_or_default();
            format!(
                "<li><strong>{}</strong> — peak {}, avg {}{}</li>",
                esc(email),
                fmt_num(*peak),
                fmt_num(*avg),
                active_note,
            )
        })
        .collect();
    let win_bar = format!(
        r#"<div class="panel" id="p-win-bar">
  <div class="panel-head" onclick="togglePanel('p-win-bar')">5-hour billing windows — peak billed tokens by user (last 30 days) <span class="chv">&#9660;</span></div>
  <div class="chart-wrap">{win_bar_svg}</div>
  <ul class="win-note">{win_bar_note}</ul>
</div>"#
    );

    // Table: all windows from the last 7 days.
    let seven_days_ago = chrono::Utc::now() - chrono::Duration::days(7);
    let win_rows: String = {
        let recent_windows: Vec<_> = windows
            .iter()
            .filter(|w| w.last_entry >= seven_days_ago)
            .collect();
        if recent_windows.is_empty() {
            r#"<tr><td colspan="9" class="empty">No window data in the last 7 days</td></tr>"#
                .to_string()
        } else {
            recent_windows
                .iter()
                .map(|w| {
                    let status = if w.is_active {
                        r#"<span class="badge ok">Active</span>"#
                    } else {
                        r#"<span class="badge">Closed</span>"#
                    };
                    format!(
                        r#"<tr>
  <td>{}</td>
  <td class="mono sm">{}</td>
  <td class="mono sm">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td class="num">{}</td><td class="num">{}</td>
  <td>{status}</td>
</tr>"#,
                        esc(&w.user_email),
                        ts(&w.start.to_rfc3339()),
                        ts(&w.end.to_rfc3339()),
                        fmt_num(w.input_tokens),
                        fmt_num(w.output_tokens),
                        fmt_num(w.cache_read_tokens),
                        fmt_num(w.cache_write_tokens),
                        w.turns,
                        w.session_count,
                    )
                })
                .collect()
        }
    };

    let win_table = format!(
        r#"<div class="panel" id="p-win-tbl">
  <div class="panel-head" onclick="togglePanel('p-win-tbl')">5-hour billing windows — last 7 days <span class="chv">&#9660;</span></div>
  <table>
    <thead><tr>
      <th>User</th><th>Window start</th><th>Window end</th>
      <th>Input</th><th>Output</th><th>Cache reads</th><th>Cache writes</th>
      <th>Turns</th><th>Sessions</th><th>Status</th>
    </tr></thead>
    <tbody>{win_rows}</tbody>
  </table>
</div>"#
    );

    let body = format!(
        r#"<div class="topbar">
  <h1>ccflux admin</h1>
  <span class="sub">Updated <span data-utc="{now_iso}">{now_display}</span></span>
  <input class="ar-inp" id="ar-inp" type="number" min="5" max="3600" value="60" onchange="onIntervalChange()" title="Refresh interval in seconds">
  <label class="ar-lbl" for="ar-inp">s</label>
  <button class="ar-btn" id="ar-btn" onclick="toggleAutoRefresh()">Auto-refresh</button>
  <span class="ar-cd" id="ar-cd"></span>
  <a class="logout-link" href="/admin/logout">Logout</a>
</div>
<div class="wrap">
  {cards}
  {chart}
  {user_bar}
  {model_bar}
  {win_bar}
  {user_table}
  {model_table}
  {win_table}
  {users_panel}
  {key_table}
  {event_table}
</div>"#
    );

    Html(page_shell("Dashboard", &body)).into_response()
}

// ── User provisioning handlers ───────────────────────────────────────────────

pub async fn handle_provision_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(admin_token) = &state.admin_token else {
        return disabled_response();
    };
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }
    if !form
        .get("csrf_token")
        .map(|t| ct_eq(t, admin_token))
        .unwrap_or(false)
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    let email = form
        .get("email")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if email.is_empty() {
        return error_page("Email is required.").into_response();
    }
    if email.len() > 254 {
        return error_page("Email address too long.").into_response();
    }
    let division = form
        .get("division")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if division.len() > 128 {
        return error_page("Division name too long.").into_response();
    }
    let days: i64 = form
        .get("expires_days")
        .and_then(|s| s.parse().ok())
        .filter(|&d: &i64| d > 0)
        .unwrap_or(365);

    match db::admin_provision_user(&state.pool, &email, &division, days).await {
        Ok(token) => Html(token_issued_page(
            &email,
            &token,
            "provisioned",
            &state.base_url,
        ))
        .into_response(),
        Err(e) => {
            eprintln!("provision_user error: {e}");
            error_page("Failed to provision user. Check server logs.").into_response()
        }
    }
}

pub async fn handle_revoke_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(admin_token) = &state.admin_token else {
        return disabled_response();
    };
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }
    if !form
        .get("csrf_token")
        .map(|t| ct_eq(t, admin_token))
        .unwrap_or(false)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(token) = form.get("token") {
        let _ = db::admin_revoke_user_token(&state.pool, token).await;
    }
    Redirect::to("/admin/").into_response()
}

pub async fn handle_reissue_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(admin_token) = &state.admin_token else {
        return disabled_response();
    };
    if !check_auth(&state, &headers) {
        return Redirect::to("/admin/login").into_response();
    }
    if !form
        .get("csrf_token")
        .map(|t| ct_eq(t, admin_token))
        .unwrap_or(false)
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    let old_token = form
        .get("token")
        .map(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if old_token.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let days: i64 = form
        .get("expires_days")
        .and_then(|s| s.parse().ok())
        .filter(|&d: &i64| d > 0)
        .unwrap_or(365);

    match db::admin_reissue_token(&state.pool, &old_token, days).await {
        Ok((email, new_token)) => Html(token_issued_page(
            &email,
            &new_token,
            "reissued",
            &state.base_url,
        ))
        .into_response(),
        Err(e) => {
            eprintln!("reissue_token error: {e}");
            error_page("Failed to reissue token. Check server logs.").into_response()
        }
    }
}

fn error_page(message: &str) -> Html<String> {
    let body = format!(
        r#"<div class="wrap" style="padding-top:2rem">
  <div class="panel">
    <div class="panel-head">Error</div>
    <div style="padding:1.25rem 1.1rem">
      <p style="color:#dc2626;margin-bottom:1rem">{}</p>
      <a href="/admin/" style="color:#4f8ef7;font-size:.85rem">← Back to dashboard</a>
    </div>
  </div>
</div>"#,
        esc(message)
    );
    Html(page_shell("Error", &body))
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

fn tier_badge(t: &crate::tiers::TierClassification) -> String {
    let (cls, title) = match t.confidence.as_str() {
        "high" => (
            "tier-badge tier-exact",
            format!(
                "Confirmed via limit event · peak ~{}",
                fmt_num(t.peak_tokens.unwrap_or(0))
            ),
        ),
        "medium" => (
            "tier-badge tier-med",
            format!(
                "Inferred · {} windows · peak ~{}",
                t.window_count,
                fmt_num(t.peak_tokens.unwrap_or(0))
            ),
        ),
        "low" => (
            "tier-badge tier-low",
            format!(
                "Inferred (low confidence) · {} windows · peak ~{}",
                t.window_count,
                fmt_num(t.peak_tokens.unwrap_or(0))
            ),
        ),
        _ => (
            "tier-badge tier-unk",
            format!("Insufficient data ({} windows)", t.window_count),
        ),
    };
    format!(
        r#"<span class="{cls}" title="{}">{}</span>"#,
        esc(&title),
        esc(&t.label)
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
    .panel-head{{padding:.65rem 1.1rem;border-bottom:1px solid #eef0f3;font-size:.78rem;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:#5a6676;cursor:pointer;user-select:none;display:flex;justify-content:space-between;align-items:center}}
    .panel.collapsed>.panel-head{{border-bottom:none}}
    .panel.collapsed>*:not(.panel-head){{display:none}}
    .chv{{font-size:.65rem;opacity:.4;transition:transform .15s;display:inline-block;flex-shrink:0}}
    .panel.collapsed .chv{{transform:rotate(-90deg)}}
    .logout-link{{color:rgba(255,255,255,.6);font-size:.82rem;text-decoration:none;margin-left:1.25rem;white-space:nowrap}}
    .logout-link:hover{{color:white}}
    .ar-inp{{background:transparent;border:1px solid rgba(255,255,255,.25);color:rgba(255,255,255,.75);font-size:.75rem;border-radius:4px;padding:.15rem .3rem;margin-left:.75rem;width:3.5rem;text-align:center;-moz-appearance:textfield}}
    .ar-inp::-webkit-inner-spin-button,.ar-inp::-webkit-outer-spin-button{{-webkit-appearance:none}}
    .ar-lbl{{font-size:.75rem;color:rgba(255,255,255,.45);margin-left:.2rem}}
    .ar-btn{{background:none;border:1px solid rgba(255,255,255,.25);color:rgba(255,255,255,.55);padding:.2rem .55rem;border-radius:4px;font-size:.78rem;cursor:pointer;margin-left:.4rem;white-space:nowrap}}
    .ar-btn.ar-on{{border-color:rgba(79,142,247,.8);color:#4f8ef7}}
    .ar-btn:hover{{color:white;border-color:rgba(255,255,255,.55)}}
    .ar-cd{{font-size:.75rem;color:rgba(255,255,255,.4);margin-left:.35rem;min-width:5rem;display:inline-block}}
    .tier-badge{{display:inline-block;padding:.15rem .45rem;border-radius:3px;font-size:.7rem;font-weight:600;letter-spacing:.04em;white-space:nowrap;cursor:default}}
    .tier-exact{{background:#d1fae5;color:#065f46}}
    .tier-med{{background:#dbeafe;color:#1e40af}}
    .tier-low{{background:#fef9c3;color:#854d0e}}
    .tier-unk{{background:#f1f5f9;color:#94a3b8}}
    .chart-wrap{{padding:1rem 1.1rem .75rem;overflow-x:auto}}
    .tbl-scroll{{overflow-y:auto;max-height:420px}}
    .tbl-scroll thead th{{position:sticky;top:0;z-index:1}}
    .tbl-filter-wrap{{padding:.5rem 1.1rem .35rem}}
    .tbl-filter{{width:100%;padding:.35rem .6rem;border:1px solid #d0d7de;border-radius:4px;font-size:.85rem;box-sizing:border-box}}
    .tbl-filter:focus{{outline:none;border-color:#4f8ef7}}
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
    .win-note{{list-style:none;padding:.75rem 1.1rem;border-top:1px solid #eef0f3;display:flex;flex-direction:column;gap:.35rem}}
    .win-note li{{font-size:.85rem;color:#1a1a2e;line-height:1.5}}
    .btn-revoke{{background:none;border:1px solid #dc2626;color:#dc2626;padding:.2rem .5rem;border-radius:4px;font-size:.72rem;cursor:pointer}}
    .btn-revoke:hover{{background:#dc2626;color:white}}
    .btn-issue{{background:none;border:1px solid #2563eb;color:#2563eb;padding:.35rem .75rem;border-radius:4px;font-size:.82rem;cursor:pointer;white-space:nowrap}}
    .btn-issue:hover{{background:#2563eb;color:white}}
    .btn-reissue{{background:none;border:1px solid #2563eb;color:#2563eb;padding:.2rem .5rem;border-radius:4px;font-size:.72rem;cursor:pointer;white-space:nowrap}}
    .btn-reissue:hover{{background:#2563eb;color:white}}
    .exp{{background:#fef3c7;color:#d97706}}
    .prov-form{{padding:.75rem 1.1rem;border-bottom:1px solid #eef0f3}}
    .prov-row{{display:flex;gap:.5rem;align-items:center;flex-wrap:wrap}}
    .prov-row input{{padding:.4rem .6rem;border:1px solid #d0d7de;border-radius:4px;font-size:.85rem;min-width:0}}
    .prov-row input[type=email]{{flex:2;min-width:160px}}
    .prov-row input[type=text]{{flex:1;min-width:100px}}
    .prov-row input[type=number]{{width:70px}}
    .prov-lbl{{font-size:.8rem;color:#5a6676;white-space:nowrap}}
    .tok-wrap{{padding:1.25rem 1.1rem}}
    .tok-warn{{color:#d97706;font-size:.85rem;margin-bottom:.75rem;font-weight:500}}
    .tok-box{{display:flex;align-items:center;gap:.75rem;margin-bottom:1rem}}
    .tok-box code{{font-family:"SFMono-Regular",Consolas,monospace;font-size:.85rem;background:#f8f9fb;padding:.5rem .75rem;border-radius:6px;border:1px solid #eef0f3;flex:1;overflow-x:auto;word-break:break-all}}
    .btn-copy{{background:none;border:1px solid #8894a4;color:#5a6676;padding:.35rem .65rem;border-radius:4px;font-size:.82rem;cursor:pointer;white-space:nowrap}}
    .btn-copy:hover{{background:#f8f9fb}}
    .tok-hint{{font-size:.85rem;color:#5a6676;line-height:1.6}}
    .back-link{{font-size:.85rem;color:#4f8ef7;text-decoration:none}}
    .back-link:hover{{text-decoration:underline}}
  </style>
</head>
<body>
{content}
<script>
document.querySelectorAll('[data-utc]').forEach(function(el){{
  var d=new Date(el.getAttribute('data-utc'));
  if(!isNaN(d.getTime()))el.textContent=d.toLocaleString(undefined,{{dateStyle:'short',timeStyle:'short'}});
}});
function filterTable(inp,tbodyId){{
  var q=inp.value.toLowerCase();
  document.getElementById(tbodyId).querySelectorAll('tr').forEach(function(row){{
    row.style.display=row.textContent.toLowerCase().indexOf(q)>=0?'':'none';
  }});
}}
function togglePanel(id){{
  var p=document.getElementById(id);
  var c=p.classList.toggle('collapsed');
  try{{localStorage.setItem('ccflux_'+id,c?'1':'0');}}catch(e){{}}
}}
document.querySelectorAll('.panel[id]').forEach(function(p){{
  try{{if(localStorage.getItem('ccflux_'+p.id)==='1')p.classList.add('collapsed');}}catch(e){{}}
}});
var arTimer=null,arLeft=0;
function arSecs(){{return Math.max(5,parseInt(document.getElementById('ar-inp').value,10)||60);}}
function arCdText(s){{return s>0?'next in '+s+'s':'';}}
function toggleAutoRefresh(){{
  var btn=document.getElementById('ar-btn');if(!btn)return;
  var on=!btn.classList.contains('ar-on');
  btn.classList.toggle('ar-on',on);
  try{{localStorage.setItem('ccflux_ar',on?'1':'0');}}catch(e){{}}
  on?arStart():arStop();
}}
function arStart(){{
  arStop();arLeft=arSecs();
  document.getElementById('ar-cd').textContent=arCdText(arLeft);
  arTimer=setInterval(function(){{
    arLeft--;
    document.getElementById('ar-cd').textContent=arCdText(arLeft);
    if(arLeft<=0){{clearInterval(arTimer);location.reload();}}
  }},1000);
}}
function arStop(){{
  clearInterval(arTimer);arTimer=null;
  var cd=document.getElementById('ar-cd');if(cd)cd.textContent='';
}}
function onIntervalChange(){{
  try{{localStorage.setItem('ccflux_ar_iv',document.getElementById('ar-inp').value);}}catch(e){{}}
  if(document.getElementById('ar-btn').classList.contains('ar-on'))arStart();
}}
(function(){{
  var btn=document.getElementById('ar-btn');if(!btn)return;
  try{{
    var iv=localStorage.getItem('ccflux_ar_iv');
    if(iv){{document.getElementById('ar-inp').value=iv;}}
    if(localStorage.getItem('ccflux_ar')==='1'){{btn.classList.add('ar-on');arStart();}}
  }}catch(e){{}}
}})();
</script>
</body>
</html>"#
    )
}

fn token_issued_page(email: &str, token: &str, action: &str, base_url: &str) -> String {
    let heading = if action == "provisioned" {
        "Token provisioned"
    } else {
        "Token reissued"
    };

    let endpoint_section = if base_url.is_empty() {
        r#"<p class="tok-hint" style="color:#f59e0b;margin-top:.75rem">
        <strong>BASE_URL not set</strong> — set the <code style="font-size:.82rem">BASE_URL</code>
        environment variable on the receiver so the endpoint appears here.
      </p>"#
            .to_string()
    } else {
        let url_esc = esc(base_url);
        let config_json = esc(&format!(
            "{{\n  \"endpoint\": \"{}\",\n  \"token\": \"{}\"\n}}",
            base_url, token
        ));
        format!(
            r#"<div class="tok-box" style="margin-top:.75rem">
        <code id="ep">{url_esc}</code>
        <button class="btn-copy"
          onclick="copyEl('ep',this)">Copy</button>
      </div>
      <p class="tok-hint" style="margin-top:.75rem">
        config.json snippet — paste both values at once:
      </p>
      <div class="tok-box" style="align-items:flex-start">
        <code id="cfg" style="white-space:pre">{config_json}</code>
        <button class="btn-copy" style="align-self:flex-start"
          onclick="copyEl('cfg',this)">Copy</button>
      </div>"#,
            url_esc = url_esc,
            config_json = config_json,
        )
    };

    let body = format!(
        r#"<div class="topbar"><h1>ccflux admin</h1></div>
<div class="wrap">
  <div class="panel">
    <div class="panel-head">{heading} — {email_esc}</div>
    <div class="tok-wrap">
      <p class="tok-warn">Copy this token now — it will not be shown again in this UI.</p>
      <p style="font-size:.82rem;color:#555;margin-bottom:.4rem">API token</p>
      <div class="tok-box">
        <code id="tok">{token_esc}</code>
        <button class="btn-copy"
          onclick="copyEl('tok',this)">Copy</button>
      </div>
      <p style="font-size:.82rem;color:#555;margin:.75rem 0 .4rem">Receiver endpoint</p>
      {endpoint_section}
      <p class="tok-hint" style="margin-top:1rem">
        Send both values to <strong>{email_esc}</strong>.<br>
        They enter them in Claude Code plugin settings or in
        <code style="font-size:.82rem">&lt;data_dir&gt;/ccflux/config.json</code>.
      </p>
    </div>
    <div style="padding:.75rem 1.1rem;border-top:1px solid #eef0f3">
      <a class="back-link" href="/admin/">&#8592; Back to dashboard</a>
    </div>
  </div>
</div>
<script>
function copyEl(id,btn){{
  var text=document.getElementById(id).textContent;
  if(navigator.clipboard){{
    navigator.clipboard.writeText(text)
      .then(function(){{btn.textContent='Copied!';setTimeout(function(){{btn.textContent='Copy';}},2000);}});
  }}else{{
    var ta=document.createElement('textarea');
    ta.value=text;ta.style.position='fixed';ta.style.opacity='0';
    document.body.appendChild(ta);ta.select();
    try{{document.execCommand('copy');btn.textContent='Copied!';}}
    catch(e){{btn.textContent='Failed';}}
    setTimeout(function(){{btn.textContent='Copy';}},2000);
    document.body.removeChild(ta);
  }}
}}
</script>"#,
        heading = heading,
        email_esc = esc(email),
        token_esc = esc(token),
        endpoint_section = endpoint_section,
    );
    page_shell(heading, &body)
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
