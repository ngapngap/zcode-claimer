import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

// ── UI claim ZCode (mỗi dòng 1 tài khoản, quota inline) ──
// 0) Đăng nhập tự động: bấm nút (topbar) -> Rust init /oauth/cli/init -> mở trình
//    duyệt mặc định cho user nhập user/pass trên web ZCode -> Rust poll
//    /oauth/cli/poll/{flow_id} -> lưu account vào accounts.json.
//    Không dùng webview nhúng trong app (mở trình duyệt ngoài).
// 1) Bảng tài khoản: mỗi dòng 1 tài khoản, quota tự tải cho mọi tài khoản khi mở
//    app — không cần bấm vào từng tài khoản để xem.
// 2) Claim per-account: nút "Lãnh gói" trên dòng -> dialog preview -> captcha slider
//    -> claim (ui/aliyun-captcha.js solve()).
// 3) Export: nút "Export JSON router" trên topbar -> export tất cả tài khoản có
//    JWT thành 1 file JSON (mảng provider_conns) để import router.

const $ = (id) => document.getElementById(id);
let captchaVerifyParam = "";
let captchaRegion = "";
let loginTimer = null;

// ── State tài khoản đang dùng (thay cho form 6 trường đã bỏ) ──
// Lưu qua Rust persistence (save_saved_config -> accounts.json, key=user_id).
let activeCfg = { device_mid: "", jwt: "", user_id: "", username: "", email: "", zai_business_token: "" };
function readCfg() { return { ...activeCfg }; }
function writeCfg(cfg) {
  activeCfg = {
    device_mid: cfg.device_mid || "",
    jwt: cfg.jwt || "",
    user_id: cfg.user_id || "",
    username: cfg.username || "",
    email: cfg.email || "",
    zai_business_token: cfg.zai_business_token || "",
  };
}
async function saveCfg() {
  try {
    await invoke("save_saved_config", { config: readCfg() });
  } catch {
    try {
      localStorage.setItem("zcode-claimer-jwt", activeCfg.jwt);
      localStorage.setItem("zcode-claimer-mid", activeCfg.device_mid);
    } catch {}
  }
}
async function ensureMid() {
  if (!activeCfg.device_mid.trim()) {
    activeCfg.device_mid = await invoke("generate_device_mid");
    await saveCfg();
  }
  return activeCfg.device_mid.trim();
}

// ── Ripple cho nút ──
document.addEventListener("click", (e) => {
  const btn = e.target.closest("button");
  if (!btn || btn.disabled) return;
  const rect = btn.getBoundingClientRect();
  const s = document.createElement("span");
  s.className = "ripple";
  const d = Math.max(rect.width, rect.height) * 2;
  s.style.width = s.style.height = d + "px";
  s.style.left = (e.clientX - rect.left - d / 2) + "px";
  s.style.top = (e.clientY - rect.top - d / 2) + "px";
  btn.appendChild(s);
  setTimeout(() => s.remove(), 600);
});
function setStep(n) {
  document.querySelectorAll(".step").forEach((s) => {
    const num = Number(s.dataset.step);
    s.classList.toggle("is-active", num === n);
    const state = s.querySelector(".step-state");
    if (state) state.textContent = num < n ? "✓" : String(num);
  });
}

// ── Đăng nhập ZCode tự động: init -> mở browser -> poll ──
$("btnLogin").onclick = async () => {
  if (loginTimer) { clearInterval(loginTimer); loginTimer = null; }
  $("loginStatus").textContent = "Đang khởi tạo đăng nhập...";
  let start;
  try {
    start = await invoke("start_zcode_login");
  } catch (e) {
    $("loginStatus").textContent = "Lỗi khởi tạo: " + e;
    return;
  }
  $("loginStatus").textContent = "Đang mở trình duyệt để đăng nhập...";
  try {
    await openUrl(start.auth_url);
  } catch (e) {
    $("loginStatus").textContent = "Mở browser thất bại: " + e + " — hãy copy địa chỉ này mở tay: " + start.auth_url;
    return;
  }
  $("loginStatus").textContent = "Đã mở browser — hãy nhập user/pass trên web ZCode để xác thực. Đang chờ bạn xác thực (poll mỗi vài giây)...";
  const deadline = (start.expires_at ? start.expires_at * 1000 : Date.now() + 5 * 60 * 1000);
  const interval = Math.max(1000, (start.poll_interval_sec || 3) * 1000);
  let attempt = 0;
  loginTimer = setInterval(async () => {
    attempt++;
    if (Date.now() > deadline) {
      clearInterval(loginTimer); loginTimer = null;
      $("loginStatus").textContent = "Hết hạn đăng nhập (5 phút). Bấm nút đăng nhập lại.";
      return;
    }
    try {
      const res = await invoke("poll_zcode_login", { flowId: start.flow_id });
      if (res?.status === "pending") return;
      clearInterval(loginTimer); loginTimer = null;
      if (res?.status === "ready" && res?.credentials) {
        const c = res.credentials;
        writeCfg({
          jwt: c.jwt || "",
          device_mid: c.device_mid || "",
          user_id: c.user_id || "",
          username: "",
          email: "",
          zai_business_token: "",
        });
        await saveCfg();
        currentUserId = c.user_id || "";
        loadAccounts();
        $("loginStatus").textContent = "Đăng nhập thành công. Username/email không lấy được từ đăng nhập web (ZCode OAuth chỉ trả user_id).";
        setStep(2);
      } else {
        $("loginStatus").textContent = "Đăng nhập thất bại: " + JSON.stringify(res);
      }
    } catch (e) {
      clearInterval(loginTimer); loginTimer = null;
      $("loginStatus").textContent = "Đăng nhập thất bại: " + e;
    }
  }, interval);
};

// ── Export JSON router: 1 nút, export tất cả tài khoản có JWT thành 1 file ──
$("btnExportAll").onclick = async () => {
  const withJwt = accountsCache.filter((a) => a.jwt && String(a.jwt).trim());
  if (!withJwt.length) {
    $("loginStatus").textContent = "Chưa có tài khoản nào có JWT để export — hãy đăng nhập trước.";
    return;
  }
  $("loginStatus").textContent = `Đang export ${withJwt.length} tài khoản...`;
  const items = [];
  let skipped = 0;
  for (const a of withJwt) {
    try {
      const s = await invoke("export_router_json", {
        jwt: a.jwt, userId: a.user_id, username: a.username || "",
        email: a.email || "", zaiBusinessToken: a.zai_business_token || "",
      });
      items.push(JSON.parse(s));
    } catch { skipped++; }
  }
  if (!items.length) {
    $("loginStatus").textContent = "Export thất bại cho tất cả tài khoản.";
    return;
  }
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([JSON.stringify(items, null, 2)], { type: "application/json" }));
  a.download = "zcode-router-export.json";
  a.click();
  $("loginStatus").textContent = `Đã export ${items.length}/${withJwt.length} tài khoản → zcode-router-export.json` + (skipped ? ` (${skipped} tài khoản lỗi, đã bỏ qua)` : "");
};

// ── Bảng tài khoản per-account (UsageCell / ActionsCell) ──
let currentUserId = "";
let accountsCache = [];
const usageCache = new Map(); // user_id -> BalanceResult | { error: string }
const usageLoading = new Set(); // user_id đang tải quota

function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
// Một định dạng số duy nhất cho toàn app (tiếng Việt, khớp nhau mọi chỗ):
// >= 1 tỷ: "x,yz tỷ" · >= 1 triệu: "x triệu" · >= 1 nghìn: "xK" · còn lại: số thường.
// (Trước đây nhánh >= 1e8 chia cho 1e8 rồi ghi "triệu" nên 500.000.000 hiện thành
// "5 triệu" — sai 100 lần so với "3M" ở chỗ khác. Giờ "triệu" luôn = 1e6.)
function trimNum(x) {
  const r = Math.round(x * 100) / 100;
  return String(r).replace(".", ",");
}
function fmtUnits(n) {
  const v = Number(n) || 0;
  if (v >= 1e9) return trimNum(v / 1e9) + " tỷ";
  if (v >= 1e6) return trimNum(v / 1e6) + " triệu";
  if (v >= 1e3) return trimNum(v / 1e3) + "K";
  return String(v);
}
function fmtTime(sec) {
  if (!sec) return "";
  try { return new Date(sec * 1000).toLocaleString("vi-VN"); } catch { return ""; }
}

// ── Cột Quota: pill xám, trên là tên gói + còn/tổng (xanh),
// dưới là thanh bar mỏng = tỉ lệ còn lại. Hover vào pill hiện popover chi tiết. ──
function primaryBucket(r) {
  const bs = (r.balances || []).filter((b) => b);
  if (!bs.length) return null;
  const withLeft = bs.filter((b) => Number(b.remaining_units) > 0);
  const pool = withLeft.length ? withLeft : bs;
  return pool.reduce((m, b) => ((Number(b.total_units) || 0) > (Number(m.total_units) || 0) ? b : m));
}

function usageCell(a) {
  if (usageLoading.has(a.user_id)) return '<span class="usage-cell muted">Đang tải quota...</span>';
  const r = usageCache.get(a.user_id);
  if (!r) return '<span class="usage-cell muted">Chưa có dữ liệu</span>';
  if (r.error) return `<span class="usage-cell err" title="${esc(r.error)}">Lỗi số dư</span>`;
  if (!r.has_quota) return '<span class="usage-cell muted">Không có hạn mức</span>';
  const b = primaryBucket(r);
  const remaining = b ? Number(b.remaining_units) : Number(r.total_remaining);
  const total = b ? Number(b.total_units) : Number(r.total_granted);
  const name = b ? (b.show_name || b.meter || b.bucket_id || "") : "Tổng";
  const pctLeft = total > 0 ? Math.max(0, Math.min(100, Math.round(remaining / total * 100))) : 0;
  const barCls = pctLeft < 10 ? "low" : "";
  return `<span class="balance-pill" data-balance-acct="${esc(a.user_id)}" tabindex="0">
    <span class="bp-top"><span class="bp-name">${esc(name)}</span><span class="bp-nums">${fmtUnits(remaining)} / ${fmtUnits(total)} token</span></span>
    <span class="bp-bar"><span class="bp-fill ${barCls}" style="width:${pctLeft}%"></span></span>
  </span>`;
}

function expiryCell(a) {
  const r = usageCache.get(a.user_id);
  if (!r || r.error) return '<span class="meta">—</span>';
  const times = [];
  (r.balances || []).forEach((b) => { if (b.period_end) times.push(b.period_end); if (b.expires_at) times.push(b.expires_at); });
  (r.plans || []).forEach((p) => { if (p.ends_at) times.push(p.ends_at); });
  if (!times.length) return '<span class="meta">—</span>';
  const soonest = Math.min(...times);
  const diff = soonest * 1000 - Date.now();
  if (diff <= 0) return '<span class="meta">Đã hết hạn</span>';
  const days = diff / 86400000;
  if (days >= 1) return `<span title="${esc(fmtTime(soonest))}">Còn ${Math.floor(days)} ngày</span>`;
  const hours = diff / 3600000;
  if (hours >= 1) return `<span title="${esc(fmtTime(soonest))}">Còn ${trimNum(hours)} giờ</span>`;
  return `<span title="${esc(fmtTime(soonest))}">Còn ${Math.max(1, Math.round(diff / 60000))} phút</span>`;
}

function renderSummary() {
  const el = $("accountsSummary");
  if (!el) return;
  const loaded = [...usageCache.values()].filter((r) => r && !r.error);
  const totalLeft = loaded.reduce((s, r) => s + (Number(r.total_remaining) || 0), 0);
  el.textContent = `ZCode · ${accountsCache.length} tài khoản · Tổng còn ${fmtUnits(totalLeft)} token`;
}

// Popover chi tiết quota khi hover vào pill (plan/status/expires,
// tổng còn lại, rồi từng bucket với bar riêng).
function balancePopHtml(a, r) {
  const b = primaryBucket(r);
  const totalLeft = Number(r.total_remaining) || 0;
  const plan = (r.plans || [])[0];
  const exp = b ? (b.expires_at || b.period_end) : (plan?.ends_at || null);
  const status = plan ? (plan.status === "active" ? "Đang hiệu lực" : plan.status === "pending" ? "Chờ hiệu lực" : plan.status) : "";
  const head = `<div class="bpp-head">Gói ${esc(b ? (b.show_name || b.meter || b.bucket_id || "") : (plan?.name || ""))}${status ? ` · ${esc(status)}` : ""}${exp ? ` · Hết hạn ${esc(fmtTime(exp))}` : ""}</div>`;
  const total = `<div class="bpp-row"><span class="muted">Tổng còn lại</span><span class="bpp-total">${fmtUnits(totalLeft)} token</span></div>`;
  const buckets = (r.balances || []).map((bk) => {
    const rem = Number(bk.remaining_units) || 0, tot = Number(bk.total_units) || 0;
    const pct = tot > 0 ? Math.max(0, Math.min(100, Math.round(rem / tot * 100))) : 0;
    return `<div class="bpp-bucket"><div class="bpp-brow"><span>${esc(bk.show_name || bk.meter || bk.bucket_id || "")}</span><span>${fmtUnits(rem)} / ${fmtUnits(tot)} token</span></div><span class="bp-bar"><span class="bp-fill" style="width:${pct}%"></span></span></div>`;
  }).join("");
  const plans = (r.plans || []).map((p) => {
    const st = p.status === "active" ? "Đang hiệu lực" : p.status === "pending" ? "Chờ hiệu lực" : (p.status || "");
    const pendingTxt = p.has_pending && p.pending_until ? " · Hiệu lực " + esc(fmtTime(p.pending_until)) : "";
    return `<div class="bpp-plan"><span>${esc(p.name || p.plan_id)}</span><span class="meta">${esc(st)}${pendingTxt}${p.ends_at ? " · đến " + esc(fmtTime(p.ends_at)) : ""}</span></div>`;
  }).join("");
  return `${head}${total}${buckets ? `<div class="bpp-title">Chi tiết</div>${buckets}` : ""}${plans ? `<div class="bpp-title">Gói</div>${plans}` : ""}`;
}
function showBalancePop(anchor, userId) {
  const pop = $("balancePop");
  const a = accountsCache.find((x) => x.user_id === userId);
  const r = usageCache.get(userId);
  if (!pop || !a || !r || r.error) return;
  pop.innerHTML = balancePopHtml(a, r);
  pop.hidden = false;
  const rc = anchor.getBoundingClientRect();
  const pw = pop.offsetWidth, ph = pop.offsetHeight;
  let left = Math.max(8, Math.min(window.innerWidth - pw - 8, rc.left));
  let top = rc.bottom + 6;
  if (top + ph > window.innerHeight - 8) top = Math.max(8, rc.top - ph - 6);
  pop.style.left = left + "px";
  pop.style.top = top + "px";
}
function hideBalancePop() { const pop = $("balancePop"); if (pop) pop.hidden = true; }

function statusCell(a) {
  const r = usageCache.get(a.user_id);
  if (r?.error) return '<span class="tag expired">Lỗi số dư</span>';
  if (!a.jwt) return '<span class="tag unknown">Chưa có JWT</span>';
  if (usageLoading.has(a.user_id)) return '<span class="tag unknown">Đang tải...</span>';
  return '<span class="tag active">Sẵn sàng</span>';
}

function actionsCell(a) {
  const hasJwt = !!(a.jwt && String(a.jwt).trim());
  const claimBtn = hasJwt
    ? `<button class="primary small" data-claim-acct="${esc(a.user_id)}" type="button" title="Xem các gói có thể lãnh của tài khoản này">Lãnh gói</button>`
    : `<button class="ghost small" type="button" disabled title="Chưa có JWT — cần đăng nhập trước">Lãnh gói</button>`;
  const editBtn = `<button class="ghost small" data-edit-email="${esc(a.user_id)}" type="button" title="Nhập/sửa email đăng nhập của tài khoản này (để nhận biết tài khoản, dùng khi export)">Sửa email</button>`;
  return `<span class="actions-cell">${claimBtn} ${editBtn}</span>`;
}

// Tên hiển thị = email đăng nhập. ZCode OAuth chỉ trả user_id (không có email),
// nên email phải nhập tay 1 lần bằng nút "Sửa email" rồi lưu vào accounts.json.
function displayName(a) {
  if (a.email && a.email.trim()) return a.email.trim();
  if (a.username && a.username.trim()) return a.username.trim();
  return a.user_id || "?";
}

function accountSub(a) {
  const parts = [];
  if (a.email) {
    if (a.user_id) parts.push(a.user_id);
    if (a.username) parts.push(a.username);
  } else if (a.username) {
    if (a.user_id) parts.push(a.user_id);
  }
  return parts.join(" · ");
}

function startEditEmail(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a) return;
  const cell = document.querySelector(`tr[data-userid="${CSS.escape(userId)}"] td.acct`);
  if (!cell) return;
  cell.innerHTML = `<input class="email-edit" type="text" value="${esc(a.email || "")}" placeholder="email đăng nhập (vd: name@gmail.com)" />
    <span class="btn-row"><button class="primary small" type="button" data-save-email>Lưu</button><button class="ghost small" type="button" data-cancel-email>Huỷ</button></span>`;
  const input = cell.querySelector(".email-edit");
  input.focus(); input.select();
  const save = async () => {
    a.email = input.value.trim();
    if (userId === currentUserId) activeCfg.email = a.email;
    try { await invoke("save_saved_config", { config: { ...a } }); } catch {}
    renderAccountsTable();
  };
  cell.querySelector("[data-save-email]").onclick = save;
  cell.querySelector("[data-cancel-email]").onclick = () => renderAccountsTable();
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") save();
    if (e.key === "Escape") renderAccountsTable();
  });
}
function shortMid(mid) {
  const m = String(mid || "").trim();
  if (!m) return '<span class="meta">—</span>';
  return `<span title="${esc(m)}">${esc(m.slice(0, 8))}…</span>`;
}

function renderAccountsTable() {
  const el = $("accountsList");
  if (!el) return;
  if (!accountsCache.length) {
    el.innerHTML = '<p class="hint">Chưa có tài khoản nào. Bấm Đăng nhập ZCode để thêm tài khoản đầu tiên.</p>';
    return;
  }
  const rows = accountsCache.map((a) => {
    const active = a.user_id === currentUserId;
    return `<tr class="account-row ${active ? "is-active" : ""}" data-userid="${esc(a.user_id)}">
      <td class="acct"><b>${esc(displayName(a))}</b>${active ? ' <span class="tag active">Đang dùng</span>' : ""}${a.email ? "" : ' <span class="tag pending">Chưa có email</span>'}<div class="meta">${esc(accountSub(a))}</div></td>
      <td>${shortMid(a.device_mid)}</td>
      <td>${usageCell(a)}</td>
      <td>${expiryCell(a)}</td>
      <td>${statusCell(a)}</td>
      <td>${actionsCell(a)}</td>
    </tr>`;
  }).join("");
  el.innerHTML = `<table class="accounts-table"><thead><tr><th class="acct">Tài khoản</th><th>Device ID</th><th>Quota</th><th>Hết hạn</th><th>Trạng thái</th><th>Thao tác</th></tr></thead><tbody>${rows}</tbody></table>`;
  renderSummary();
  document.querySelectorAll("[data-claim-acct]").forEach((btn) => {
    btn.onclick = () => openPlansForAccount(btn.dataset.claimAcct);
  });
  document.querySelectorAll("[data-edit-email]").forEach((btn) => {
    btn.onclick = () => startEditEmail(btn.dataset.editEmail);
  });
  document.querySelectorAll(".account-row").forEach((tr) => {
    tr.onclick = (e) => {
      if (e.target.closest("button")) return;
      switchAccount(tr.dataset.userid);
    };
  });
}

// Hover pill quota -> popover chi tiết (delegate, vì bảng render lại bằng innerHTML)
$("accountsList").addEventListener("mouseover", (e) => {
  const pill = e.target.closest(".balance-pill");
  if (pill) showBalancePop(pill, pill.dataset.balanceAcct);
});
$("accountsList").addEventListener("mouseout", (e) => {
  const pill = e.target.closest(".balance-pill");
  const pop = $("balancePop");
  if (pill && pop && !pop.contains(e.relatedTarget) && !pill.contains(e.relatedTarget)) hideBalancePop();
});
$("balancePop").addEventListener("mouseleave", hideBalancePop);
document.addEventListener("click", (e) => {
  const pop = $("balancePop");
  if (pop && !pop.hidden && !e.target.closest(".balance-pill") && !pop.contains(e.target)) hideBalancePop();
});

async function queryUsageOnce(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a || !a.jwt) return;
  const mid = (a.device_mid || "").trim() || (userId === currentUserId ? await ensureMid() : "");
  if (!mid) {
    usageCache.set(userId, { error: "Chưa có Device ID", has_quota: false, balances: [], plans: [], total_remaining: 0, total_used: 0, total_granted: 0 });
    renderAccountsTable();
    return;
  }
  usageLoading.add(userId);
  renderAccountsTable();
  try {
    const r = await invoke("query_balance", { jwt: a.jwt, deviceMid: mid });
    usageCache.set(userId, r);
  } catch (e) {
    // Lỗi bắn ra cột Quota của chính account đó (không chỉ quota card chính)
    usageCache.set(userId, { error: String(e), has_quota: false, balances: [], plans: [], total_remaining: 0, total_used: 0, total_granted: 0 });
  } finally {
    usageLoading.delete(userId);
    renderAccountsTable();
  }
}

// ── Dialog Lãnh gói per-account (ActionsCell → openZcodePlans → wbZcodeClaim) ──
async function openPlansForAccount(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a) return;
  // Tự chuyển account đang dùng trước khi mở dialog để không vô vòng
  writeCfg(a);
  currentUserId = userId;
  await saveCfg();
  renderAccountsTable();
  // Sau đây dialog lấy jwt/mid từ snapshot a (không đọc phần tử form đã đổi)
  openPlansDialog(a);
}

function openPlansDialog(a) {
  const dlg = $("plansDialog");
  if (!dlg) return;
  dlg.hidden = false;
  $("plansDialogAccount").textContent = `Tài khoản: ${displayName(a)} (${a.user_id})`;
  $("plansDialogBody").innerHTML = '<p class="hint">Đang tải các gói...</p>';
  $("plansDialogResult").textContent = "";
  $("btnClosePlans").onclick = () => { dlg.hidden = true; };
  dlg.onclick = (e) => { if (e.target === dlg) dlg.hidden = true; };
  // preview không captcha nên gọi thẳng
  (async () => {
    const mid = (a.device_mid || "").trim() || await ensureMid();
    try {
      const result = await invoke("preview_plans", { jwt: a.jwt, deviceMid: mid });
      if (result?.NotDeployed) { $("plansDialogBody").innerHTML = '<p class="hint">Hoạt động chưa ra mắt (404) — chờ mở bán</p>'; return; }
      const plans = result?.Plans || [];
      if (!plans.length) { $("plansDialogBody").innerHTML = '<p class="hint">Không có gói nào hôm nay</p>'; return; }
      let dialogPlanId = null;
      $("plansDialogBody").innerHTML = plans.map((p) =>
        `<div class="plan-item" data-dialog-id="${esc(p.plan_id)}"><span><b>${esc(p.name)}</b> — ${esc(p.plan_id)}<br><span style="font-size:12px;color:#6e6e73">${esc(p.description || "")}</span></span><span>${p.entitlements?.[0] ? fmtUnits(p.entitlements[0].grant_units) + " " + esc(p.entitlements[0].unit_type || "") : ""}</span></div>`
      ).join("") + `<div class="btn-row"><button id="btnDialogCaptcha" class="secondary" type="button">Làm captcha (slider)</button><button id="btnDialogClaim" class="primary" type="button" disabled>Lãnh gói</button></div>`;
      dlg.querySelectorAll(".plan-item").forEach((el) => {
        el.onclick = () => {
          dlg.querySelectorAll(".plan-item").forEach((x) => x.classList.remove("selected"));
          el.classList.add("selected");
          dialogPlanId = el.dataset.dialogId;
          $("btnDialogCaptcha").disabled = false;
        };
      });
      $("btnDialogCaptcha").onclick = async () => {
        if (!dialogPlanId) { $("plansDialogResult").textContent = "Chưa chọn gói"; return; }
        let config;
        try { config = await invoke("get_captcha_config"); } catch (e) { $("plansDialogResult").textContent = "Lấy cấu hình captcha lỗi: " + e; return; }
        if (!config || !config.enabled) { $("plansDialogResult").textContent = "Upstream nói không cần mã xác thực — bấm Lãnh luôn"; $("btnDialogClaim").disabled = false; return; }
        $("plansDialogResult").textContent = "Kéo slider để hoàn thành captcha...";
        try {
          await window.wbAliyunCaptcha.solve(
            { region: config.region, prefix: config.prefix, sceneId: config.scene_id },
            async (verifyParam) => {
              captchaVerifyParam = verifyParam;
              captchaRegion = config.region;
              $("plansDialogResult").textContent = "Captcha OK — bấm Lãnh gói";
              $("btnDialogClaim").disabled = false;
              return { captchaResult: true, bizResult: true };
            }
          );
        } catch (e) {
          if (e?.name === "CaptchaCancelledError") { $("plansDialogResult").textContent = "Đã hủy captcha"; return; }
          $("plansDialogResult").textContent = "Lỗi captcha: " + (e?.message || e);
        }
      };
      $("btnDialogClaim").onclick = async () => {
        if (!dialogPlanId) { $("plansDialogResult").textContent = "Chưa chọn gói"; return; }
        $("plansDialogResult").textContent = "Đang lãnh...";
        try {
          const result = await invoke("claim_plan", { jwt: a.jwt, deviceMid: mid, planId: dialogPlanId, captchaVerifyParam, captchaRegion });
          if (result?.Claimed) {
            $("plansDialogResult").textContent = "Đã lãnh: " + result.Claimed.plan_id;
            setStep(3);
            try { await invoke("record_claim_history", { planId: result.Claimed.plan_id, date: new Date().toISOString().slice(0, 10), outcome: "claimed", code: 0, message: "", failureEndsAt: null }); } catch {}
            queryUsageOnce(userId);
          } else if (result?.Failed) {
            const hint = retryHint(result.Failed.code, result.Failed.failure_ends_at);
            $("plansDialogResult").textContent = `Lỗi: ${result.Failed.failure}: ${result.Failed.message} (code ${result.Failed.code})` + (hint ? ` — ${hint.text}` : "");
            try { await invoke("record_claim_history", { planId: dialogPlanId, date: new Date().toISOString().slice(0, 10), outcome: "failed", code: result.Failed.code, message: result.Failed.message || "", failureEndsAt: result.Failed.failure_ends_at ?? null }); } catch {}
            if (result.Failed.code === 3007) $("btnDialogClaim").disabled = true;
            queryUsageOnce(userId);
          }
        } catch (e) { $("plansDialogResult").textContent = "Lỗi: " + e; }
      };
    } catch (e) { $("plansDialogBody").innerHTML = `<p class="hint" style="color:#a50e0e">Lỗi preview: ${esc(String(e))}</p>`; }
  })();
}

async function switchAccount(userId) {
  const cfg = accountsCache.find((x) => x.user_id === userId);
  if (!cfg) return;
  writeCfg(cfg);
  currentUserId = userId;
  await saveCfg();
  renderAccountsTable();
  queryUsageOnce(userId);
}

async function loadAccounts() {
  const el = $("accountsList");
  if (!el) return;
  try {
    accountsCache = (await invoke("list_accounts")) || [];
    if (!accountsCache.length) {
      renderAccountsTable();
      return;
    }
    if (!currentUserId) currentUserId = accountsCache[0]?.user_id || "";
    if (!accountsCache.some((a) => a.user_id === currentUserId)) currentUserId = accountsCache[0].user_id;
    renderAccountsTable();
    // Tự load số dư cho mọi tài khoản (hiện thẳng vào cột Quota, không cần bấm)
    for (const a of accountsCache) {
      if (a.jwt) { queryUsageOnce(a.user_id); }
    }
  } catch (e) {
    el.textContent = "Không tải được danh sách tài khoản: " + e;
  }
}

// Gợi ý retry nhẹ theo code (khớp ClaimFailure::retry_action ở backend):
// 1005 -> chờ tới failure_ends_at; 3007 -> làm lại captcha slider; 401 -> login lại.
function retryHint(code, failureEndsAt) {
  if (code === 1005) {
    const when = failureEndsAt ? ` (đến ${fmtTime(failureEndsAt)})` : "";
    return { cls: "warn", text: `Hạn mức hôm nay đã hết (1005). Hãy chờ tới${when} rồi claim lại.` };
  }
  if (code === 3007) return { cls: "warn", text: "Captcha chưa đạt (3007). Hãy làm lại slider rồi bấm Lãnh gói." };
  if (code === 401) return { cls: "err", text: "Phiên đăng nhập đã hết hạn (401). Hãy đăng nhập lại lấy JWT mới." };
  if (code === 1003) return { cls: "ok", text: "Tài khoản này đã nhận gói hôm nay (1003)." };
  return null;
}

$("btnRefreshAll")?.addEventListener("click", async () => {
  for (const a of accountsCache) { if (a.jwt) await queryUsageOnce(a.user_id); }
});

// ── Persistence Rust thay localStorage JWT ──
// Ưu tiên load_saved_config → save_saved_config. Chỉ khi Tauri không sẵn sàng
// (mở bằng trình duyệt thuần cho dev) mới fallback localStorage.
let tauriReady = null;
async function isTauri() {
  if (!tauriReady) {
    tauriReady = invoke("load_saved_config")
      .then(cfg => { writeCfg(cfg); return true; })
      .catch(() => {
        // Fallback dev (trình duyệt thuần): localStorage — không giữ cho release.
        try {
          activeCfg.jwt = localStorage.getItem("zcode-claimer-jwt") || "";
          activeCfg.device_mid = localStorage.getItem("zcode-claimer-mid") || "";
        } catch {}
        return false;
      });
  }
  return tauriReady;
}

// ── Cập nhật tự động (Tauri updater): check khi mở app (silent) + nút check tay ──
// Manifest (updates/latest.json) lấy từ repo raw; updater plugin tự tải .sig/.msi/.exe,
// kiểm tra chữ ký bằng pubkey nhúng trong tauri.conf.json, cài đặt xong relaunch.
// Luồng chính thức: check() -> update/downloadAndInstall() -> relaunch() (process plugin).
let pendingUpdate = null;
async function checkForUpdates(manual) {
  if (!isTauri()) return;
  const st = $("updateStatus");
  if (manual) st.textContent = "Đang kiểm tra...";
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    pendingUpdate = update || null;
    $("btnInstallUpdate").hidden = !update;
    if (update) {
      st.textContent = `Có bản mới v${update.version} — bấm "Tải & cài đặt" để cập nhật.`;
      if (update.body) st.textContent += `\n${update.body}`;
    } else {
      st.textContent = manual ? "Bản hiện tại đã mới nhất." : "Bản hiện tại đã mới nhất.";
    }
  } catch (e) {
    if (manual) st.textContent = "Lỗi kiểm tra: " + e;
  }
}
$("btnCheckUpdate").onclick = () => checkForUpdates(true);
$("btnInstallUpdate").onclick = async () => {
  if (!pendingUpdate) return;
  const st = $("updateStatus"), prog = $("updateProgress");
  $("btnInstallUpdate").disabled = true;
  prog.hidden = false;
  st.textContent = "Đang tải bản cập nhật...";
  try {
    let total = 0, downloaded = 0;
    await pendingUpdate.downloadAndInstall(async (event) => {
      if (event.event === "Started") { total = event.contentLength || 0; }
      if (event.event === "Progress") { downloaded += event.chunkLength; prog.value = total ? Math.round(100 * downloaded / total) : 0; }
    });
    st.textContent = "Cài đặt xong, đang khởi động lại...";
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch (e) {
    st.textContent = "Lỗi cài đặt cập nhật: " + e;
    $("btnInstallUpdate").disabled = false;
  }
};

// Dữ liệu giả để xem thử giao diện (?demo=1, chỉ khi mở bằng trình duyệt thuần;
// app thật chạy Tauri không có query này nên không ảnh hưởng).
function seedDemo() {
  const now = Math.floor(Date.now() / 1000);
  accountsCache = [
    { user_id: "11111111-1111-4111-8111-111111111111", username: "demo-zcode-01", email: "demo01@example.com", device_mid: "22222222-2222-4222-8222-222222222222", jwt: "demo", zai_business_token: "" },
    { user_id: "33333333-3333-4333-8333-333333333333", username: "demo-zcode-02", email: "", device_mid: "44444444-4444-4444-8444-444444444444", jwt: "demo", zai_business_token: "" },
    { user_id: "55555555-5555-4555-8555-555555555555", username: "demo-zcode-03", email: "demo03@example.com", device_mid: "66666666-6666-4666-8666-666666666666", jwt: "demo", zai_business_token: "" },
  ];
  currentUserId = accountsCache[0].user_id;
  usageCache.set(accountsCache[0].user_id, {
    total_remaining: 3000000, total_used: 2000000, total_granted: 5000000, has_quota: true,
    balances: [
      { show_name: "GLM-5.3", meter: "tokens", unit_type: "tokens", total_units: 3000000, used_units: 0, remaining_units: 3000000, period_end: now + 86400 * 7, expires_at: now + 86400 * 30 },
      { show_name: "GLM-5.3-Flash", meter: "tokens", unit_type: "tokens", total_units: 500000000, used_units: 500000000, remaining_units: 0, period_end: now + 86400 * 3, expires_at: now + 86400 * 10 },
    ],
    plans: [
      { plan_id: "zcode-v3-start-plan-0817", name: "ZCode Start Plan", status: "active", ends_at: now + 86400 * 7, has_pending: false, pending_until: null },
      { plan_id: "zcode-v3-weekend-1106-wk", name: "ZCODE WEEKEND BUILD", status: "pending", ends_at: now + 86400, has_pending: true, pending_until: now + 86400 },
    ],
  });
  usageCache.set(accountsCache[1].user_id, {
    total_remaining: 414000000, total_used: 86000000, total_granted: 500000000, has_quota: true,
    balances: [
      { show_name: "ZCODE WEEKEND BUILD", meter: "tokens", unit_type: "tokens", total_units: 500000000, used_units: 86000000, remaining_units: 414000000, period_end: now + 86400 * 2, expires_at: now + 86400 * 2 },
    ],
    plans: [
      { plan_id: "zcode-v3-weekend-1106-wk", name: "ZCODE WEEKEND BUILD", status: "active", ends_at: now + 86400 * 2, has_pending: false, pending_until: null },
    ],
  });
  usageCache.set(accountsCache[2].user_id, {
    has_quota: false, balances: [], plans: [],
    total_remaining: 0, total_used: 0, total_granted: 0,
  });
  renderAccountsTable();
}

// Khởi tạo: nạp config đã lưu từ Rust persistence khi mở app
isTauri().then(() => {
  if (new URLSearchParams(location.search).has("demo")) { seedDemo(); return; }
  loadAccounts();
  // Auto-check update khi mở app (silent, không popup dialog — dialog:false trong tauri.conf.json)
  checkForUpdates(false);
});
