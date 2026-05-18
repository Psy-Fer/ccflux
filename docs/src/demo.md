# Live Demo

Read-only snapshot of the admin dashboard, populated with 15 sample users across four seat tiers and 45 days of usage history. All charts, search filters, and panel expand/collapse work — action buttons are removed.

<div style="margin:1.25rem 0">
  <a href="./dashboard.html" target="_blank" style="font-size:.9rem">Open in full page ↗</a>
</div>

<div style="border:1px solid #d0d4da;border-radius:6px;overflow:hidden;margin:1rem 0">
  <iframe src="./dashboard.html" width="100%" height="820" style="border:none;display:block" title="ccflux admin dashboard demo"></iframe>
</div>

---

To export a snapshot from your own instance:

```bash
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
  "https://ccflux.example.org/admin/?export" \
  -o dashboard-snapshot.html
```
