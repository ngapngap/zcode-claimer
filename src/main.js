import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { save } from "@tauri-apps/plugin-dialog";

// ── UI claim ZCode (mỗi dòng 1 tài khoản, quota inline) ──
// 0) Đăng nhập tự động: bấm nút (topbar) -> Rust init /oauth/cli/init -> mở trình
//    duyệt mặc định cho user nhập user/pass trên web ZCode -> Rust poll
//    /oauth/cli/poll/{flow_id} -> lưu account vào accounts.json.
//    Không dùng webview nhúng trong app (mở trình duyệt ngoài).
// 1) Bảng tài khoản: mỗi dòng 1 tài khoản, quota tự tải cho mọi tài khoản khi mở
//    app — không cần bấm vào từng tài khoản để xem.
// 2) Claim per-account: nút "Nhận gói" trên dòng -> dialog preview -> captcha slider
//    -> claim (ui/aliyun-captcha.js solve()).
// 3) Export: nút "Export JSON router" trên topbar -> export tất cả tài khoản
//    (ZCode có JWT + AutoClaw có token) thành 1 file JSON (mảng provider_conns)
//    để import router.

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

// ── Magnetic UI (bản B, libraries.dev): nút primary dịch nhẹ theo con trỏ ──
// Tối đa 3px, chỉ khi user không bật prefers-reduced-motion. Dùng delegate
// nên nút tạo động (Nhận gói, Xem điểm...) cũng có hiệu ứng.
if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
  document.addEventListener("mousemove", (e) => {
    const btn = e.target.closest?.("button.primary");
    if (!btn || btn.disabled) return;
    const r = btn.getBoundingClientRect();
    const dx = (e.clientX - (r.left + r.width / 2)) / r.width;
    const dy = (e.clientY - (r.top + r.height / 2)) / r.height;
    btn.style.transform = `translate(${(dx * 3).toFixed(1)}px,${(dy * 2).toFixed(1)}px)`;
  });
  document.addEventListener("mouseout", (e) => {
    const btn = e.target.closest?.("button.primary");
    if (btn) btn.style.transform = "";
  });
}
function setStep(n) {
  document.querySelectorAll(".step").forEach((s) => {
    const num = Number(s.dataset.step);
    s.classList.toggle("is-active", num === n);
    s.classList.toggle("is-done", num < n);
    const state = s.querySelector(".step-state");
    if (state) state.textContent = num < n ? "✓" : String(num);
  });
  // Vạch nối giữa các bước: xong khi bước trước nó đã xong.
  document.querySelectorAll(".step-sep").forEach((sep, i) => {
    sep.classList.toggle("done", n > i + 1);
  });
}

// ── Đăng nhập ZCode tự động: init -> mở browser -> poll ──
// Combobox loại đăng nhập: ZCode (OAuth web, như cũ) hoặc AutoClaw
// (đọc login AutoClaw desktop local, chỉ Quốc tế — Trung Quốc đã bỏ).
function loginType() { return ($("loginType")?.value === "autoclaw") ? "autoclaw" : "zcode"; }
function refreshLoginLabel() {
  const el = $("btnLoginLabel");
  if (el) el.textContent = loginType() === "autoclaw" ? "Đăng nhập AutoClaw" : "Đăng nhập ZCode";
}
$("loginType")?.addEventListener("change", refreshLoginLabel);
refreshLoginLabel();

$("btnLogin").onclick = async () => {
  if (loginType() === "autoclaw") { await loginAutoclaw(); return; }
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
    // start có thể là null nếu lệnh init trả về rỗng (demo không có Tauri):
    // không đọc lại start.auth_url ở đây nữa, không thì chính catch này ném tiếp.
    $("loginStatus").textContent = "Mở browser thất bại: " + e + (start?.auth_url ? " — hãy copy địa chỉ này mở tay: " + start.auth_url : "");
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

// Lưu file export: app thật thì hỏi đường dẫn qua hộp thoại native (plugin-dialog),
// rồi ghi file bằng lệnh Rust save_text_file; trình duyệt thường/demo thì tải kiểu cũ.
async function downloadExportFile(filename, contents) {
  try {
    const path = await save({
      title: "Lưu file export JSON router",
      defaultPath: filename,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return { cancelled: true };
    await invoke("save_text_file", { path, contents });
    return { path };
  } catch {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([contents], { type: "application/json" }));
    a.download = filename;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
    return { downloaded: true };
  }
}

// ── Export JSON router: 1 nút, export tất cả tài khoản (ZCode có JWT +
// AutoClaw có token) thành 1 file ──
$("btnExportAll").onclick = async () => {
  const withJwt = accountsCache.filter((a) => a.jwt && String(a.jwt).trim());
  const withAcToken = accountsCache.filter((a) => a.type === "autoclaw" && a.autoclaw_token && String(a.autoclaw_token).trim());
  const total = withJwt.length + withAcToken.length;
  if (!total) {
    $("loginStatus").textContent = "Chưa có tài khoản nào để export — hãy đăng nhập trước.";
    return;
  }
  $("loginStatus").textContent = `Đang export ${total} tài khoản...`;
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
  for (const a of withAcToken) {
    try {
      const s = await invoke("export_router_json_autoclaw", {
        token: a.autoclaw_token, refreshToken: a.autoclaw_refresh_token || "",
        userId: acRawId(a.user_id), username: a.username || "",
        email: a.email || "",
      });
      items.push(JSON.parse(s));
    } catch { skipped++; }
  }
  if (!items.length) {
    $("loginStatus").textContent = "Export thất bại cho tất cả tài khoản.";
    return;
  }
  const res = await downloadExportFile("zcode-router-export.json", JSON.stringify(items, null, 2));
  if (res.cancelled) {
    $("loginStatus").textContent = "Đã hủy export.";
    return;
  }
  const dest = res.path ? `→ ${res.path}` : "→ zcode-router-export.json (tải về thư mục mặc định)";
  $("loginStatus").textContent = `Đã export ${items.length}/${total} tài khoản ${dest}` + (skipped ? ` (${skipped} tài khoản lỗi, đã bỏ qua)` : "");
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

// ── Cột Quota: hiện LUÔN tất cả bucket, mỗi bucket 1 dòng (tên + còn/tổng + bar mỏng).
// Không còn popover hover — mọi thông tin đã nằm ngay trong ô. ──
function usageCell(a) {
  if (usageLoading.has(a.user_id)) return '<span class="usage-cell muted">Đang tải quota...</span>';
  const r = usageCache.get(a.user_id);
  if (!r) return '<span class="usage-cell muted">Chưa có dữ liệu</span>';
  if (r.error) return `<span class="usage-cell err" title="${esc(r.error)}">Lỗi số dư</span>`;
  if (!r.has_quota) return '<span class="usage-cell muted">Không có hạn mức</span>';
  const bs = (r.balances || []).filter((b) => b);
  if (!bs.length) return '<span class="usage-cell muted">Không có hạn mức</span>';
  const lines = bs.map((b) => {
    const remaining = Number(b.remaining_units) || 0;
    const total = Number(b.total_units) || 0;
    const name = b.show_name || b.meter || b.bucket_id || "";
    const pctLeft = total > 0 ? Math.max(0, Math.min(100, Math.round(remaining / total * 100))) : 0;
    const barCls = pctLeft < 10 ? "low" : "";
    const numsCls = remaining <= 0 ? "zero" : "";
    return `<div class="usage-line"><div class="bp-top"><span class="bp-name" title="${esc(name)}">${esc(name)}</span><span class="bp-nums ${numsCls}">${fmtUnits(remaining)} / ${fmtUnits(total)}</span></div><span class="bp-bar"><span class="bp-fill ${barCls}" style="width:${pctLeft}%"></span></span></div>`;
  }).join("");
  return `<div class="usage-lines quota">${lines}</div>`;
}

// Text hết hạn của AutoClaw (hiện thành 1 dòng nhỏ dưới các dòng điểm).
// API trả text gốc tiếng Trung (vd "积分将于3天内到期") — không hiện nguyên:
// chỉ lấy số ngày từ đó, còn lại dịch hết sang tiếng Việt. Trả "" nếu không có.
function autoclawExpiryText(a) {
  const r = pointsCache.get(a.user_id);
  const exp = r?.expiring || null;
  const pts = exp != null && exp.points != null ? Number(exp.points) : NaN;
  const hasPts = Number.isFinite(pts);
  const m = exp && typeof exp.text === "string" ? exp.text.match(/(\d+)\s*天/) : null;
  const days = m ? Number(m[1]) : NaN;
  if (hasPts && Number.isFinite(days)) return `Sắp hết hạn: ${fmtNum(pts)} điểm trong ${days} ngày`;
  if (hasPts) return `Sắp hết hạn: ${fmtNum(pts)} điểm`;
  if (Number.isFinite(days)) return `Sắp hết hạn trong ${days} ngày`;
  return "";
}

function renderSummary() {
  const zcodeAccts = accountsCache.filter((a) => a.type !== "autoclaw");
  const acAccts = accountsCache.filter((a) => a.type === "autoclaw");
  const loaded = [...usageCache.values()].filter((r) => r && !r.error);
  const totalLeft = loaded.reduce((s, r) => s + (Number(r.total_remaining) || 0), 0);
  const totalGranted = loaded.reduce((s, r) => s + (Number(r.total_granted) || 0), 0);
  const acPoints = acAccts
    .map((a) => pointsCache.get(a.user_id))
    .filter((r) => r && !r.error && Number.isFinite(Number(r.available)))
    .reduce((s, r) => s + Number(r.available), 0);
  // Tóm tắt ở tiêu đề từng cụm (vẫn thấy khi cụm thu gọn) + stat strip 3 ô.
  const set = (id, v) => { const n = $(id); if (n) n.textContent = v; };
  set("statTokens", fmtUnits(totalLeft));
  set("statTokensSub", `${zcodeAccts.length} tài khoản · cấp ${fmtUnits(totalGranted)}`);
  set("statPoints", acAccts.length ? fmtNum(acPoints) : "—");
  // Subtitle Điểm AutoClaw: lấy tên ví thật đang tải được (qua walletNameVi),
  // loại trùng — không hardcode tên ví nào nữa.
  const acWalletNames = [...new Set(
    acAccts
      .map((a) => pointsCache.get(a.user_id))
      .filter((r) => r && Array.isArray(r.wallets))
      .flatMap((r) => r.wallets.map((w) => walletNameVi(w.show_name || w.bucket_id) || "Điểm"))
  )];
  set("statPointsSub", acAccts.length ? (acWalletNames.length ? acWalletNames.join(" · ") : "—") : "Chưa có tài khoản AutoClaw");
  set("zcodeSummary", zcodeAccts.length
    ? `${zcodeAccts.length} tài khoản · Token còn ${fmtUnits(totalLeft)} / ${fmtUnits(totalGranted)}`
    : "Chưa có tài khoản ZCode");
  set("autoclawSummary", acAccts.length
    ? `${acAccts.length} tài khoản · ${fmtNum(acPoints)} điểm`
    : "Chưa có tài khoản AutoClaw");
}

function statusCell(a) {
  // Trạng thái đợt "Nhận tất cả" (nếu có): ưu tiên hiện trạng thái đợt
  // thay cho trạng thái quota thường, cho tới khi làm mới lại.
  const bc = batchClaim.get(a.user_id);
  if (bc) {
    if (bc.kind === "running") return '<span class="tag unknown">Đang nhận...</span>';
    if (bc.kind === "done") return `<span class="tag active" title="${esc(bc.planId || "")}">Đã nhận</span>`;
    if (bc.kind === "error") return `<span class="tag expired">Lỗi nhận</span><div class="meta" style="color:#b91c1c">${esc(bc.text)}</div>`;
    if (bc.kind === "skipped") return `<span class="tag unknown" title="${esc(bc.text)}">Đã bỏ qua</span>`;
  }
  if (a.type === "autoclaw") {
    const r = pointsCache.get(a.user_id);
    if (r?.expired) return '<span class="tag expired" title="Token đã hết hạn — đăng nhập lại AutoClaw">Hết hạn đăng nhập</span>';
    if (r?.error) return '<span class="tag expired">Lỗi số dư</span>';
    if (r?.loading) return '<span class="tag unknown">Đang tải...</span>';
    if (r) return '<span class="tag active">Sẵn sàng</span>';
    return '<span class="tag unknown">Chưa xem điểm</span>';
  }
  const r = usageCache.get(a.user_id);
  if (r?.error) return '<span class="tag expired">Lỗi số dư</span>';
  if (!a.jwt) return '<span class="tag unknown">Chưa có JWT</span>';
  if (usageLoading.has(a.user_id)) return '<span class="tag unknown">Đang tải...</span>';
  if (!r) return '<span class="tag unknown">Chưa tải</span>';
  if (!r.has_quota) return '<span class="tag unknown">Hết hạn mức</span>';
  return '<span class="tag active">Sẵn sàng</span>';
}

// Menu "…" trên mỗi dòng: Sửa email / Xóa (ít dùng) gom vào đây;
// nút chính (Nhận gói / Điểm danh / Xem điểm) giữ nguyên ngoài bảng.
function rowMenuBtns(a) {
  return `<span class="row-menu"><button class="ghost small row-menu-btn" data-menu-acct="${esc(a.user_id)}" type="button" aria-haspopup="true" aria-expanded="false" title="Thêm thao tác"><svg viewBox="0 0 24 24" width="15" height="15" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg></button><span class="row-menu-pop" hidden><button type="button" data-edit-email="${esc(a.user_id)}">Sửa email</button><button type="button" class="danger" data-delete-acct="${esc(a.user_id)}">Xóa tài khoản</button></span></span>`;
}

function actionsCell(a) {
  if (a.type === "autoclaw") {
    // AutoClaw không có JWT ZCode nên không Nhận gói. Sửa email được
    // (lưu vào autoclaw-accounts.json, không chạm accounts.json ZCode).
    // Điểm danh hàng ngày: task daily_signin (xem autoclaw_checkin.rs) —
    // nút đổi trạng thái trong phiên sau khi nhận xong.
    const ck = checkinState.get(a.user_id);
    const checkinBtn = checkinRunning.has(a.user_id)
      ? `<button class="primary small" type="button" disabled>Đang điểm danh...</button>`
      : ck?.done
        ? `<button class="ghost small" type="button" disabled title="${esc(ck.msg || "Hôm nay đã điểm danh")}">Đã điểm danh${ck.reward ? ` +${fmtNum(ck.reward)}` : ""}</button>`
        : `<button class="primary small" data-checkin-acct="${esc(a.user_id)}" type="button" title="Điểm danh hàng ngày nhận điểm AutoClaw">Điểm danh</button>`;
    const viewBtn = `<button class="primary small" data-points-acct="${esc(a.user_id)}" type="button" title="Tải lại số điểm AutoClaw">Xem điểm</button>`;
    return `<span class="actions-cell">${checkinBtn} ${viewBtn} ${rowMenuBtns(a)}</span>`;
  }
  const hasJwt = !!(a.jwt && String(a.jwt).trim());
  const claimBtn = hasJwt
    ? `<button class="primary small" data-claim-acct="${esc(a.user_id)}" type="button" title="Xem các gói có thể nhận của tài khoản này"${batchRunning ? " disabled" : ""}>Nhận gói</button>`
    : `<button class="ghost small" type="button" disabled title="Chưa có JWT — cần đăng nhập trước">Nhận gói</button>`;
  return `<span class="actions-cell">${claimBtn} ${rowMenuBtns(a)}</span>`;
}

// Tên hiển thị = email đăng nhập. ZCode OAuth chỉ trả user_id (không có email),
// nên email phải nhập tay 1 lần bằng nút "Sửa email" rồi lưu vào accounts.json.
function displayName(a) {
  if (a.email && a.email.trim()) return a.email.trim();
  if (a.username && a.username.trim()) return a.username.trim();
  return a.user_id || "?";
}

// Dòng phụ dưới tên tài khoản: chỉ hiện username khi nó thêm thông tin
// so với tên hiển thị (email). User ID / Device ID dài không hiện ở đây
// nữa — xem bằng tooltip (title) của ô tài khoản.
function accountSub(a) {
  if (a.email && a.username && a.username.trim() && a.username.trim() !== a.email.trim()) return a.username.trim();
  return "";
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
    if (a.type === "autoclaw") {
      // Lưu email vào autoclaw-accounts.json (giữ nguyên token đã lưu).
      try {
        await invoke("save_autoclaw_account", { account: { user_id: acRawId(userId), user_name: a.username || "", email: a.email, token: a.autoclaw_token || "", refresh_token: a.autoclaw_refresh_token || "", device_id: a.autoclaw_device_id || "", expires_at: a.autoclaw_expires_at ?? null } });
      } catch {}
    } else {
      try { await invoke("save_saved_config", { config: { ...a } }); } catch {}
    }
    renderAccountsTable();
  };
  cell.querySelector("[data-save-email]").onclick = save;
  cell.querySelector("[data-cancel-email]").onclick = () => renderAccountsTable();
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") save();
    if (e.key === "Escape") renderAccountsTable();
  });
}
// Xóa 1 tài khoản đã lưu: hỏi chắc chắn rồi mới xóa khỏi accounts.json
// (qua lệnh Rust delete_account). Xóa xong cập nhật bảng + summary ngay;
// nếu xóa đúng tài khoản đang dùng thì reset cả config đang dùng.
async function deleteAccount(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a) return;
  const label = displayName(a);
  if (a.type === "autoclaw") {
    // Xóa cả file autoclaw-accounts.json (nếu tài khoản này đã lưu);
    // login AutoClaw desktop không bị ảnh hưởng.
    if (!window.confirm(`Xóa tài khoản AutoClaw ${label}?\nToken đã lưu sẽ mất. Muốn dùng lại thì chọn AutoClaw rồi bấm Đăng nhập.`)) return;
    try { await invoke("delete_autoclaw_account", { userId: acRawId(userId) }); } catch {}
    accountsCache = accountsCache.filter((x) => x.user_id !== userId);
    pointsCache.delete(userId);
    renderAccountsTable();
    const st = $("loginStatus");
    if (st) st.textContent = `Đã xóa tài khoản AutoClaw ${label}.`;
    return;
  }
  if (!window.confirm(`Xóa tài khoản ${label}?\nJWT và device ID đã lưu sẽ mất. Muốn dùng lại phải đăng nhập lại.`)) return;
  try { await invoke("delete_account", { userId }); } catch {}
  accountsCache = accountsCache.filter((x) => x.user_id !== userId);
  usageCache.delete(userId);
  if (userId === currentUserId) { currentUserId = ""; writeCfg({}); }
  renderAccountsTable();
  const st = $("loginStatus");
  if (st) st.textContent = `Đã xóa tài khoản ${label}.`;
}

// 2 bảng riêng: ZCode (Tài khoản · Quota · Trạng thái · Thao tác) và
// AutoClaw (Tài khoản · Điểm · Trạng thái · Thao tác). Mỗi tài khoản 1 dòng,
// không trộn 2 loại vào chung 1 bảng nữa.
function accountRowHtml(a) {
  const active = a.user_id === currentUserId;
  const isAc = a.type === "autoclaw";
  const sub = accountSub(a);
  const title = `User ID: ${a.user_id}` + (!isAc && a.device_mid ? ` · Device ID: ${a.device_mid}` : "");
  return `<tr class="account-row ${active ? "is-active" : ""}" data-userid="${esc(a.user_id)}">
      <td class="acct" title="${esc(title)}"><b>${esc(displayName(a))}</b>${active ? ' <span class="tag active">Đang dùng</span>' : ""}${!isAc && !a.email ? ' <span class="tag pending">Chưa có email</span>' : ""}${sub ? `<div class="meta">${esc(sub)}</div>` : ""}</td>
      <td>${isAc ? pointsCell(a) : usageCell(a)}</td>
      <td>${statusCell(a)}</td>
      <td>${actionsCell(a)}</td>
    </tr>`;
}

function renderAccountsTable() {
  const zEl = $("zcodeList");
  const acEl = $("autoclawList");
  if (!zEl || !acEl) return;
  closeAllRowMenus();
  const zcodeAccts = accountsCache.filter((a) => a.type !== "autoclaw");
  const acAccts = accountsCache.filter((a) => a.type === "autoclaw");
  zEl.innerHTML = zcodeAccts.length
    ? `<table class="accounts-table"><thead><tr><th class="acct">Tài khoản</th><th>Quota</th><th>Trạng thái</th><th>Thao tác</th></tr></thead><tbody>${zcodeAccts.map(accountRowHtml).join("")}</tbody></table>`
    : '<p class="hint">Chưa có tài khoản ZCode nào. Chọn ZCode rồi bấm Đăng nhập để thêm tài khoản đầu tiên.</p>';
  acEl.innerHTML = acAccts.length
    ? `<table class="accounts-table"><thead><tr><th class="acct">Tài khoản</th><th>Điểm</th><th>Trạng thái</th><th>Thao tác</th></tr></thead><tbody>${acAccts.map(accountRowHtml).join("")}</tbody></table>`
    : '<p class="hint">Chưa có tài khoản AutoClaw nào. Chọn AutoClaw rồi bấm Đăng nhập để thêm tài khoản đầu tiên.</p>';
  renderSummary();
  document.querySelectorAll("[data-claim-acct]").forEach((btn) => {
    btn.onclick = () => openPlansForAccount(btn.dataset.claimAcct);
  });
  document.querySelectorAll("[data-edit-email]").forEach((btn) => {
    btn.onclick = () => { closeAllRowMenus(); startEditEmail(btn.dataset.editEmail); };
  });
  document.querySelectorAll("[data-delete-acct]").forEach((btn) => {
    btn.onclick = () => { closeAllRowMenus(); deleteAccount(btn.dataset.deleteAcct); };
  });
  document.querySelectorAll("[data-points-acct]").forEach((btn) => {
    btn.onclick = () => queryPointsOnce(btn.dataset.pointsAcct);
  });
  document.querySelectorAll("[data-checkin-acct]").forEach((btn) => {
    btn.onclick = () => checkinAutoclawOnce(btn.dataset.checkinAcct);
  });
  document.querySelectorAll("[data-menu-acct]").forEach((btn) => {
    btn.onclick = (e) => {
      e.stopPropagation();
      const pop = btn.parentElement.querySelector(".row-menu-pop");
      const wasHidden = !pop || pop.hidden;
      closeAllRowMenus();
      if (wasHidden && pop) { pop.hidden = false; btn.setAttribute("aria-expanded", "true"); }
    };
  });
  document.querySelectorAll(".account-row").forEach((tr) => {
    tr.onclick = (e) => {
      if (e.target.closest("button")) return;
      // Dòng AutoClaw không chuyển tài khoản ZCode đang dùng
      // (nó không có JWT/device ZCode).
      const a = accountsCache.find((x) => x.user_id === tr.dataset.userid);
      if (a?.type === "autoclaw") return;
      switchAccount(tr.dataset.userid);
    };
  });
}

// Đóng mọi menu "…" đang mở (gọi khi render lại bảng, bấm ra ngoài, Esc).
function closeAllRowMenus() {
  document.querySelectorAll(".row-menu-pop").forEach((p) => { p.hidden = true; });
  document.querySelectorAll("[data-menu-acct]").forEach((b) => b.setAttribute("aria-expanded", "false"));
}
document.addEventListener("click", (e) => { if (!e.target.closest(".row-menu")) closeAllRowMenus(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeAllRowMenus(); });

// ── Thu gọn / mở rộng từng cụm (ZCode / AutoClaw), nhớ bằng localStorage ──
// Thu gọn chỉ ẩn phần thân thẻ (bảng) — tiêu đề, tổng số và nút thao tác
// (Nhận tất cả / Làm mới tất cả) vẫn thấy được. Ẩn/hiện chỉ là xem:
// "Nhận tất cả" vẫn nhận cho mọi tài khoản ZCode dù thẻ đang thu gọn.
function applyCollapse(which, collapsed, save = true) {
  const card = $(which === "zcode" ? "zcodeCard" : "autoclawCard");
  const btn = document.querySelector(`[data-collapse="${which}"]`);
  if (!card || !btn) return;
  card.classList.toggle("is-collapsed", collapsed);
  btn.setAttribute("aria-expanded", String(!collapsed));
  const label = btn.querySelector(".collapse-label");
  if (label) label.textContent = collapsed ? "Mở rộng" : "Thu gọn";
  if (save) { try { localStorage.setItem(`zc.ui.collapsed.${which}`, collapsed ? "1" : "0"); } catch {} }
}
function initCollapse() {
  ["zcode", "autoclaw"].forEach((which) => {
    let collapsed = false;
    try { collapsed = localStorage.getItem(`zc.ui.collapsed.${which}`) === "1"; } catch {}
    applyCollapse(which, collapsed, false);
    const btn = document.querySelector(`[data-collapse="${which}"]`);
    if (btn) btn.onclick = () => {
      const card = $(which === "zcode" ? "zcodeCard" : "autoclawCard");
      applyCollapse(which, !card.classList.contains("is-collapsed"));
    };
  });
}
initCollapse();

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

// ── Dialog Nhận gói per-account (ActionsCell → openZcodePlans → wbZcodeClaim) ──
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
  // Backend (m02612): preview_plans(jwt, device_mid, user_id) -> {plans, deployed, activated, activationError}
  // - preview rỗng lần 1 -> tự báo cáo app_launch/app_daily_active rồi preview lại 1 lần
  // - report lỗi chỉ ghi vào activated/activationError, không chặn preview
  (async () => {
    const mid = (a.device_mid || "").trim() || await ensureMid();
    try {
      const result = await invoke("preview_plans", { jwt: a.jwt, deviceMid: mid, userId: a.user_id || "" });
      if (result?.deployed === false) { $("plansDialogBody").innerHTML = '<p class="hint">Hoạt động chưa ra mắt (404) — chờ mở bán</p>'; return; }
      const plans = result?.plans || [];
      if (result?.activationError) {
        $("plansDialogResult").textContent = `Báo cáo kích hoạt (app_daily_active) lỗi: ${result.activationError} — đã thử preview lại`;
      }
      if (!plans.length) { $("plansDialogBody").innerHTML = '<p class="hint">Không có gói nào hôm nay</p>'; return; }
      let dialogPlanId = null;
      $("plansDialogBody").innerHTML = plans.map((p) =>
        `<div class="plan-item" data-dialog-id="${esc(p.plan_id)}"><span><b>${esc(p.name)}</b> — ${esc(p.plan_id)}<br><span style="font-size:12px;color:#6e6e73">${esc(p.description || "")}</span></span><span>${p.entitlements?.[0] ? fmtUnits(p.entitlements[0].grant_units) + " " + esc(p.entitlements[0].unit_type || "") : ""}</span></div>`
      ).join("") + `<div class="btn-row"><button id="btnDialogCaptcha" class="secondary" type="button">Làm captcha (slider)</button><button id="btnDialogClaim" class="primary" type="button" disabled>Nhận gói</button></div>`;
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
        if (!config || !config.enabled) { $("plansDialogResult").textContent = "Upstream nói không cần mã xác thực — bấm Nhận luôn"; $("btnDialogClaim").disabled = false; return; }
        $("plansDialogResult").textContent = "Kéo slider để hoàn thành captcha...";
        try {
          await window.wbAliyunCaptcha.solve(
            { region: config.region, prefix: config.prefix, sceneId: config.scene_id },
            async (verifyParam) => {
              captchaVerifyParam = verifyParam;
              captchaRegion = config.region;
              $("plansDialogResult").textContent = "Captcha OK — bấm Nhận gói";
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
        $("plansDialogResult").textContent = "Đang nhận...";
        try {
          const result = await invoke("claim_plan", { jwt: a.jwt, deviceMid: mid, userId: a.user_id || "", planId: dialogPlanId, captchaVerifyParam, captchaRegion });
          if (result?.Claimed) {
            $("plansDialogResult").textContent = "Đã nhận: " + result.Claimed.plan_id;
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
  const el = $("zcodeList");
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
  if (code === 3007) return { cls: "warn", text: "Captcha chưa đạt (3007). Hãy làm lại slider rồi bấm Nhận gói." };
  if (code === 401) return { cls: "err", text: "Phiên đăng nhập đã hết hạn (401). Hãy đăng nhập lại lấy JWT mới." };
  if (code === 1003) return { cls: "ok", text: "Tài khoản này đã nhận gói hôm nay (1003)." };
  return null;
}

// ── Nhận tất cả: nhận lần lượt từng tài khoản ZCode, tuần tự ──
// Dùng đúng luồng nhận theo dòng (preview → captcha Aliyun → claim →
// làm mới quota) như dialog Nhận gói (openPlansDialog), chỉ khác:
// không mở dialog, gói lấy gói đầu tiên trong preview, captcha giải inline.
// Dòng AutoClaw không có gói nên không nhận; dòng không JWT hoặc đã hết
// hạn mức thì bỏ qua (không gọi API). Lỗi 1 dòng không dừng cả đợt.
// Chế độ demo (?demo=1): mô phỏng thành công sau 1 nhịp chờ để kiểm tra
// được bằng trình duyệt (không có Tauri, không gọi API thật).
function isDemoMode() { return new URLSearchParams(location.search).has("demo"); }
const batchClaim = new Map(); // user_id -> { kind: running|done|error|skipped, text?, planId? }
let batchRunning = false;

async function claimAccountOnce(a) {
  if (isDemoMode()) {
    await new Promise((r) => setTimeout(r, 600));
    setStep(3);
    return { planId: "zcode-v3-start-plan-0817", demo: true };
  }
  const mid = (a.device_mid || "").trim() || await ensureMid();
  const result = await invoke("preview_plans", { jwt: a.jwt, deviceMid: mid, userId: a.user_id || "" });
  if (result?.deployed === false) throw new Error("Hoạt động chưa ra mắt (404) — chờ mở bán");
  const plans = result?.plans || [];
  if (!plans.length) throw new Error("Không có gói nào hôm nay");
  const plan = plans[0];
  let config;
  try { config = await invoke("get_captcha_config"); } catch (e) { throw new Error("Lấy cấu hình captcha lỗi: " + e); }
  if (config && config.enabled) {
    try {
      await window.wbAliyunCaptcha.solve(
        { region: config.region, prefix: config.prefix, sceneId: config.scene_id },
        async (verifyParam) => {
          captchaVerifyParam = verifyParam;
          captchaRegion = config.region;
          return { captchaResult: true, bizResult: true };
        }
      );
    } catch (e) {
      if (e?.name === "CaptchaCancelledError") throw new Error("Đã hủy captcha");
      throw new Error("Lỗi captcha: " + (e?.message || e));
    }
  }
  const res = await invoke("claim_plan", { jwt: a.jwt, deviceMid: mid, userId: a.user_id || "", planId: plan.plan_id, captchaVerifyParam, captchaRegion });
  if (res?.Claimed) {
    try { await invoke("record_claim_history", { planId: res.Claimed.plan_id, date: new Date().toISOString().slice(0, 10), outcome: "claimed", code: 0, message: "", failureEndsAt: null }); } catch {}
    setStep(3);
    await queryUsageOnce(a.user_id);
    return { planId: res.Claimed.plan_id };
  }
  if (res?.Failed) {
    const hint = retryHint(res.Failed.code, res.Failed.failure_ends_at);
    try { await invoke("record_claim_history", { planId: plan.plan_id, date: new Date().toISOString().slice(0, 10), outcome: "failed", code: res.Failed.code, message: res.Failed.message || "", failureEndsAt: res.Failed.failure_ends_at ?? null }); } catch {}
    await queryUsageOnce(a.user_id);
    throw new Error(`Lỗi: ${res.Failed.failure}: ${res.Failed.message} (code ${res.Failed.code})` + (hint ? ` — ${hint.text}` : ""));
  }
  throw new Error("Kết quả nhận gói không rõ: " + JSON.stringify(res));
}

async function claimAll() {
  if (batchRunning) return;
  const zcodeAccts = accountsCache.filter((a) => a.type !== "autoclaw");
  const st = $("loginStatus");
  if (!zcodeAccts.length) {
    if (st) st.textContent = "Chưa có tài khoản ZCode nào để nhận.";
    return;
  }
  // Đóng dialog Nhận gói nếu đang mở để 2 luồng không nhận chồng nhau.
  const dlg = $("plansDialog");
  if (dlg) dlg.hidden = true;
  batchRunning = true;
  batchClaim.clear();
  $("btnClaimAll").disabled = true;
  renderAccountsTable();
  let ok = 0, fail = 0, skipped = 0;
  for (const a of zcodeAccts) {
    const r = usageCache.get(a.user_id);
    if (!a.jwt || !String(a.jwt).trim()) {
      batchClaim.set(a.user_id, { kind: "skipped", text: "Chưa có JWT — cần đăng nhập trước" });
      skipped++;
      renderAccountsTable();
      continue;
    }
    if (r && r.has_quota === false) {
      batchClaim.set(a.user_id, { kind: "skipped", text: "Hết hạn mức" });
      skipped++;
      renderAccountsTable();
      continue;
    }
    batchClaim.set(a.user_id, { kind: "running", text: "Đang nhận..." });
    renderAccountsTable();
    try {
      const res = await claimAccountOnce(a);
      batchClaim.set(a.user_id, { kind: "done", text: "Đã nhận", planId: res.planId });
      ok++;
    } catch (e) {
      batchClaim.set(a.user_id, { kind: "error", text: String(e?.message || e) });
      fail++;
    }
    renderAccountsTable();
  }
  batchRunning = false;
  $("btnClaimAll").disabled = false;
  renderAccountsTable();
  const parts = [];
  if (fail) parts.push(`${fail} lỗi`);
  if (skipped) parts.push(`${skipped} bỏ qua`);
  if (st) st.textContent = `Đã nhận ${ok}/${zcodeAccts.length} tài khoản` + (parts.length ? ` (${parts.join(", ")})` : "");
}

$("btnClaimAll").onclick = () => { claimAll(); };

$("btnRefreshAll")?.addEventListener("click", async () => {
  batchClaim.clear();
  for (const a of accountsCache) {
    if (a.type === "autoclaw") await queryPointsOnce(a.user_id);
    else if (a.jwt) await queryUsageOnce(a.user_id);
  }
});

// ── AutoClaw: 1 tài khoản = 1 dòng trong bảng (điểm, chỉ Quốc tế) ──
// Đăng nhập AutoClaw Quốc tế (OAuth Zai):
//   captcha-config -> slider captcha -> oauth-url -> mở trình duyệt
//   -> callback localhost:<port đăng ký> -> oauth-login -> token.
// Token lưu vào autoclaw-accounts.json (file riêng, không trộn accounts.json
// của ZCode) nên mở app lại vẫn còn tài khoản. Tài khoản thêm bằng đường
// desktop-import (đọc login AutoClaw desktop) thì chỉ sống trong phiên.
const pointsCache = new Map(); // user_id -> AutoClawBalanceResult | { error }
// Điểm danh hàng ngày: user_id -> {done, reward, msg} (trạng thái trong phiên;
// server chỉ tính 1 lần/ngày nên bấm lại cũng không nhận trùng).
const checkinState = new Map();
const checkinRunning = new Set(); // user_id đang điểm danh (chống bấm 2 lần)
let autoclawLoginTimer = null;
let autoclawLoginFlowId = null;
function fmtNum(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return "—";
  return n.toLocaleString("vi-VN");
}
// user_id trong file lưu là id thô; trên giao diện thêm tiền tố autoclaw:
// để không đụng user_id ZCode (cùng 1 tài khoản Zhipu có thể vừa ZCode vừa AutoClaw).
function acRawId(userId) { return String(userId || "").replace(/^autoclaw:/, ""); }
async function queryPointsOnce(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a || a.type !== "autoclaw") return;
  pointsCache.set(userId, { loading: true });
  renderAccountsTable();
  try {
    // Ưu tiên token đã lưu (OAuth); trống thì đọc login desktop (fallback).
    // Token AutoClaw hết hạn sau ~1 ngày: backend tự refresh từ Ủy quyền
    // (refresh_token) trước/sau khi gọi balance (xem autoclaw_refresh.rs),
    // nên KHÔNG chặn ở đây nữa — để token hết hạn đi vào backend để refresh.
    // Hạn (expires_at theo ms, Rust: exp * 1000) chỉ để hiển thị trạng thái
    // trong cột Trạng thái, không chặn gọi API.
    const r = await invoke("query_autoclaw_balance", { token: a.autoclaw_token || "", refreshToken: a.autoclaw_refresh_token || null, deviceId: a.autoclaw_device_id || null, region: "intl", userId: acRawId(userId) });
    pointsCache.set(userId, r);
    // Backend có thể đã refresh access token mới (hết hạn/~1 ngày): đồng bộ
    // lại vào tài khoản đã lưu để lần mở app sau dùng token mới ngay, và giữ
    // token/refreshToken mới trong phiên cho lần gọi tiếp theo.
    if (r?._refreshed) {
      if (r._refreshed.token) a.autoclaw_token = r._refreshed.token;
      if (r._refreshed.refresh_token) a.autoclaw_refresh_token = r._refreshed.refresh_token;
      if (r._refreshed.expires_at != null) a.autoclaw_expires_at = r._refreshed.expires_at;
      try { await invoke("save_autoclaw_account", { account: { user_id: acRawId(userId), user_name: a.username || "", email: a.email || "", token: a.autoclaw_token || "", refresh_token: a.autoclaw_refresh_token || "", device_id: a.autoclaw_device_id || "", expires_at: a.autoclaw_expires_at ?? null } }); } catch {}
    }
  } catch (e) {
    pointsCache.set(userId, { error: String(e) });
  }
  renderAccountsTable();
}
// Ngày Bắc Kinh ("YYYY-MM-DD") để đối chiếu grant_date trong sổ điểm
// (quà hoạt động tính theo ngày Bắc Kinh, xem autoclaw_checkin.rs).
function beijingToday() {
  try { return new Date().toLocaleDateString("en-CA", { timeZone: "Asia/Shanghai" }); }
  catch { return new Date().toISOString().slice(0, 10); }
}

// Điểm danh hàng ngày 1 tài khoản AutoClaw (task daily_signin):
// thành công thì làm mới điểm ngay; đã điểm danh rồi thì nút đổi trạng thái
// trong phiên. Chế độ demo (?demo=1): mô phỏng thành công +500 điểm.
async function checkinAutoclawOnce(userId) {
  const a = accountsCache.find((x) => x.user_id === userId);
  if (!a || a.type !== "autoclaw") return;
  if (checkinRunning.has(userId)) return;
  if (isDemoMode()) {
    checkinRunning.add(userId);
    renderAccountsTable();
    await new Promise((r) => setTimeout(r, 600));
    checkinRunning.delete(userId);
    const reward = 500;
    checkinState.set(userId, { done: true, reward, msg: `Điểm danh thành công, +${fmtNum(reward)} điểm` });
    const r = pointsCache.get(userId);
    if (r && Number.isFinite(Number(r.available))) {
      r.available = Number(r.available) + reward;
      const w = Array.isArray(r.wallets) ? r.wallets[0] : null;
      if (w) w.remaining_points = Number(w.remaining_points || 0) + reward;
    }
    renderAccountsTable();
    $("loginStatus").textContent = `Điểm danh thành công, +${fmtNum(reward)} điểm (demo).`;
    return;
  }
  checkinRunning.add(userId);
  renderAccountsTable();
  $("loginStatus").textContent = "Đang điểm danh...";
  try {
    const res = await invoke("autoclaw_daily_checkin", { token: a.autoclaw_token || "", refreshToken: a.autoclaw_refresh_token || null, deviceId: a.autoclaw_device_id || null, region: "intl", userId: acRawId(userId), today: beijingToday() });
    checkinRunning.delete(userId);
    // Backend tự refresh access token hết hạn (m02688): đồng bộ token mới
    // vào tài khoản đã lưu để lần mở app sau không cần login lại.
    if (res?._refreshed) {
      if (res._refreshed.token) a.autoclaw_token = res._refreshed.token;
      if (res._refreshed.refresh_token) a.autoclaw_refresh_token = res._refreshed.refresh_token;
      if (res._refreshed.expires_at != null) a.autoclaw_expires_at = res._refreshed.expires_at;
      try { await invoke("save_autoclaw_account", { account: { user_id: acRawId(userId), user_name: a.username || "", email: a.email || "", token: a.autoclaw_token || "", refresh_token: a.autoclaw_refresh_token || "", device_id: a.autoclaw_device_id || "", expires_at: a.autoclaw_expires_at ?? null } }); } catch {}
    }
    const reward = Number(res?.rewardPoints) || 0;
    const promo = res?.promotionReward;
    const promoPts = promo?.granted ? (Number(promo.points) || 0) : 0;
    const done = res?.success === true || res?.alreadyCompleted === true;
    let msg;
    if (res?.reason === "claimed") msg = `Điểm danh thành công, +${fmtNum(reward)} điểm`;
    else if (res?.reason === "already") msg = "Hôm nay đã điểm danh";
    else if (res?.reason === "failed") msg = `Điểm danh thất bại: ${res?.msg || ""}`.trim();
    else msg = res?.taskHint ? `Điểm danh chưa nhận (${res.taskHint})` : "Điểm danh chưa nhận";
    if (promoPts > 0) msg += ` · Quà hoạt động +${fmtNum(promoPts)} đã về`;
    if (done) checkinState.set(userId, { done: true, reward, msg });
    $("loginStatus").textContent = msg;
    renderAccountsTable();
    if (res?.success) await queryPointsOnce(userId);
  } catch (e) {
    checkinRunning.delete(userId);
    renderAccountsTable();
    $("loginStatus").textContent = "Điểm danh lỗi: " + e;
  }
}
// Đưa credential OAuth vào bảng + lưu file (creds: {token, refreshToken?,
// userName?, userId, deviceId, expiresAt?} từ Rust oauth-login).
async function upsertAutoclawAccount(creds) {
  const rawId = (creds.userId && String(creds.userId).trim()) || "oauth";
  const userId = "autoclaw:" + rawId;
  let a = accountsCache.find((x) => x.user_id === userId);
  const record = {
    user_id: rawId,
    user_name: (creds.userName || "").trim(),
    email: a?.email || "",
    token: creds.token || "",
    refresh_token: creds.refreshToken || "",
    device_id: creds.deviceId || "",
    expires_at: creds.expiresAt ?? null,
  };
  if (!a) {
    a = { type: "autoclaw", user_id: userId, username: record.user_name, email: "", device_mid: "", jwt: "", zai_business_token: "", autoclaw_token: record.token, autoclaw_refresh_token: record.refresh_token, autoclaw_device_id: record.device_id, autoclaw_expires_at: record.expires_at };
    accountsCache.push(a);
  } else {
    a.type = "autoclaw";
    if (record.user_name) a.username = record.user_name;
    a.autoclaw_token = record.token;
    a.autoclaw_refresh_token = record.refresh_token;
    a.autoclaw_device_id = record.device_id;
    a.autoclaw_expires_at = record.expires_at;
  }
  try { await invoke("save_autoclaw_account", { account: record }); } catch {}
  await queryPointsOnce(userId);
  return a;
}
// Đăng nhập AutoClaw Quốc tế (OAuth Zai):
// captcha -> lấy auth URL -> mở trình duyệt -> poll chờ callback.
async function loginAutoclaw() {
  if (autoclawLoginTimer) { clearTimeout(autoclawLoginTimer); autoclawLoginTimer = null; }
  if (autoclawLoginFlowId) {
    try { await invoke("cancel_autoclaw_oauth", { flowId: autoclawLoginFlowId }); } catch {}
    autoclawLoginFlowId = null;
  }
  $("loginStatus").textContent = "Đang lấy cấu hình captcha AutoClaw Quốc tế...";
  let config;
  try {
    config = await invoke("autoclaw_oauth_captcha_config");
  } catch (e) {
    $("loginStatus").textContent = "Lấy cấu hình captcha lỗi: " + e;
    return;
  }
  if (!config || !config.enabled) {
    // Quốc tế lẽ ra luôn enabled; nếu upstream nói không cần captcha thì
    // quay về đường đọc login desktop (không cần đăng nhập lại).
    $("loginStatus").textContent = "AutoClaw Quốc tế không yêu cầu captcha — đang đọc login desktop...";
    await loginAutoclawDesktop();
    return;
  }
  $("loginStatus").textContent = "Kéo slider để hoàn thành captcha...";
  // Lỗi ở bước start_oauth (vd hết cổng callback) phải hiện đúng tên của nó,
  // không gắn nhãn "Lỗi captcha" — nên bắt riêng trong callback, ghi vào
  // startError rồi báo sau khi solve xong (solve có bọc lại lỗi thì cờ này
  // vẫn còn, không bị mất).
  let startError = null;
  try {
    await window.wbAliyunCaptcha.solve(
      { region: config.region, prefix: config.prefix, sceneId: config.sceneId },
      async (verifyParam) => {
        try {
          const start = await invoke("start_autoclaw_oauth", { vendor: "zai", captchaVerifyParam: verifyParam });
          autoclawLoginFlowId = start.flow_id;
          try {
            await openUrl(start.auth_url);
          } catch (e) {
            $("loginStatus").textContent = "Mở trình duyệt thất bại: " + e + " — hãy copy địa chỉ này mở tay: " + start.auth_url;
          }
          $("loginStatus").textContent = "Đã mở trình duyệt — hãy đăng nhập Zai trên web AutoClaw, rồi quay lại app. Đang chờ xác thực...";
        } catch (e) {
          startError = e;
        }
        return { captchaResult: true, bizResult: true };
      }
    );
  } catch (e) {
    if (startError) {
      $("loginStatus").textContent = "Lỗi khởi tạo đăng nhập AutoClaw: " + (startError?.message || startError);
      return;
    }
    if (e?.name === "CaptchaCancelledError") { $("loginStatus").textContent = "Đã hủy captcha"; return; }
    $("loginStatus").textContent = "Lỗi captcha: " + (e?.message || e);
    return;
  }
  if (startError) {
    $("loginStatus").textContent = "Lỗi khởi tạo đăng nhập AutoClaw: " + (startError?.message || startError);
    return;
  }
  if (!autoclawLoginFlowId) {
    $("loginStatus").textContent = "Không khởi tạo được phiên đăng nhập AutoClaw.";
    return;
  }
  const flowId = autoclawLoginFlowId;
  const deadline = Date.now() + 5 * 60 * 1000;
  // Poll nối tiếp bằng setTimeout (không dùng setInterval + async để các
  // tick không chồng nhau: tick sau chỉ chạy khi tick trước đã xong, nên
  // không có tick muộn nào ghi đè "Đăng nhập thành công".
  let pollDone = false;
  const stopPoll = () => { if (autoclawLoginTimer) { clearTimeout(autoclawLoginTimer); autoclawLoginTimer = null; } };
  const tick = async () => {
    if (pollDone) return;
    if (Date.now() > deadline) {
      pollDone = true; stopPoll();
      try { await invoke("cancel_autoclaw_oauth", { flowId }); } catch {}
      autoclawLoginFlowId = null;
      $("loginStatus").textContent = "Hết hạn đăng nhập AutoClaw (5 phút). Bấm Đăng nhập lại.";
      return;
    }
    let res;
    try {
      res = await invoke("poll_autoclaw_oauth", { flowId });
    } catch (e) {
      if (pollDone) return;
      pollDone = true; stopPoll();
      autoclawLoginFlowId = null;
      $("loginStatus").textContent = "Đăng nhập AutoClaw thất bại: " + e;
      return;
    }
    if (pollDone) return;
    if (res?.status === "pending") {
      if (res.hint) $("loginStatus").textContent = res.hint;
      autoclawLoginTimer = setTimeout(tick, 1000);
      return;
    }
    pollDone = true; stopPoll();
    autoclawLoginFlowId = null;
    if (res?.status === "ready" && res?.credentials) {
      const a = await upsertAutoclawAccount(res.credentials);
      $("loginStatus").textContent = `Đăng nhập AutoClaw thành công${a.email ? " (" + a.email + ")" : ""}. Bấm Xem điểm ở dòng tài khoản để tải lại điểm.`;
    } else {
      $("loginStatus").textContent = "Đăng nhập AutoClaw thất bại: " + (res?.message || JSON.stringify(res));
    }
  };
  autoclawLoginTimer = setTimeout(tick, 1000);
}
// Đường cũ: đọc login AutoClaw desktop local rồi thêm 1 dòng (chỉ trong phiên).
// Dùng khi upstream nói không cần captcha, hoặc làm fallback.
async function loginAutoclawDesktop() {
  $("loginStatus").textContent = "Đang đọc login AutoClaw desktop (chỉ Quốc tế)...";
  let r;
  try {
    r = await invoke("query_autoclaw_balance", { token: "", refreshToken: null, deviceId: null, region: "intl" });
  } catch (e) {
    $("loginStatus").textContent = "Đăng nhập AutoClaw thất bại: " + e;
    return;
  }
  const userId = "autoclaw:" + ((r.user_id && String(r.user_id).trim()) || "desktop");
  let a = accountsCache.find((x) => x.user_id === userId);
  if (!a) {
    a = { type: "autoclaw", user_id: userId, username: "", email: (r.email || "").trim(), device_mid: "", jwt: "", zai_business_token: "", autoclaw_token: "", autoclaw_refresh_token: "", autoclaw_device_id: "", autoclaw_expires_at: null };
    accountsCache.push(a);
  } else {
    a.type = "autoclaw";
    if (r.email) a.email = String(r.email).trim();
  }
  pointsCache.set(userId, r);
  renderAccountsTable();
  $("loginStatus").textContent = `Đã thêm tài khoản AutoClaw${a.email ? " (" + a.email + ")" : ""} — ${fmtNum(r.available ?? 0)} điểm. Tài khoản này chỉ sống trong phiên (đọc từ login desktop), mở app lại thì đăng nhập lại.`;
}
// Nạp các tài khoản AutoClaw đã lưu (OAuth) khi mở app.
async function loadAutoclawAccounts() {
  let list;
  try {
    list = await invoke("list_autoclaw_accounts");
  } catch {
    return;
  }
  if (!Array.isArray(list)) return;
  for (const rec of list) {
    const rawId = (rec.user_id && String(rec.user_id).trim()) || "";
    if (!rawId) continue;
    const userId = "autoclaw:" + rawId;
    if (accountsCache.some((x) => x.user_id === userId)) continue;
    accountsCache.push({ type: "autoclaw", user_id: userId, username: (rec.user_name || "").trim(), email: (rec.email || "").trim(), device_mid: "", jwt: "", zai_business_token: "", autoclaw_token: (rec.token || "").trim(), autoclaw_refresh_token: (rec.refresh_token || "").trim(), autoclaw_device_id: (rec.device_id || "").trim(), autoclaw_expires_at: rec.expires_at ?? null });
  }
  renderAccountsTable();
  for (const rec of list) {
    const rawId = (rec.user_id && String(rec.user_id).trim()) || "";
    if (rawId) await queryPointsOnce("autoclaw:" + rawId);
  }
}
// Tên ví API Zai trả về tiếng Trung (vd "奖励积分") — dịch sang tiếng Việt khi
// hiển thị. Tên nào không có trong bảng thì giữ nguyên (không làm mất dữ liệu).
// Đường legacy đã dùng nhãn tiếng Việt sẵn (WALLET_SCOPE_LABELS bên Rust).
const WALLET_NAME_VI = {
  "奖励积分": "Điểm thưởng",
  "套餐积分": "Điểm gói",
  "赠送积分": "Điểm tặng",
};
function walletNameVi(name) {
  const raw = String(name ?? "").trim();
  return WALLET_NAME_VI[raw] || raw;
}
function pointsCell(a) {
  const r = pointsCache.get(a.user_id);
  if (r?.loading) return '<span class="usage-cell muted">Đang tải điểm...</span>';
  if (!r) return '<span class="usage-cell muted">Chưa xem điểm</span>';
  if (r.expired) return '<span class="usage-cell err" title="Token đã hết hạn — đăng nhập lại AutoClaw để xem điểm">Hết hạn đăng nhập — đăng nhập lại</span>';
  if (r.error) return `<span class="usage-cell err" title="${esc(r.error)}">Lỗi số dư</span>`;
  const avail = Number(r.available);
  const wallets = Array.isArray(r.wallets) ? r.wallets : [];
  if (!wallets.length) {
    const expTxt = autoclawExpiryText(a);
    const expLine = expTxt ? `<div class="exp-line">${esc(expTxt)}</div>` : "";
    return `<div class="usage-lines points"><div class="usage-line"><div class="bp-top"><span class="bp-name">Điểm</span><span class="bp-nums">${fmtNum(Number.isFinite(avail) ? avail : 0)} điểm</span></div></div>${expLine}</div>`;
  }
  const lines = wallets.map((w) => {
    const name = walletNameVi(w.show_name || w.bucket_id) || "Điểm";
    const pts = Number(w.remaining_points) || 0;
    return `<div class="usage-line"><div class="bp-top"><span class="bp-name">${esc(name)}</span><span class="bp-nums">${fmtNum(pts)} điểm</span></div></div>`;
  }).join("");
  const expTxt = autoclawExpiryText(a);
  const expLine = expTxt ? `<div class="exp-line">${esc(expTxt)}</div>` : "";
  return `<div class="usage-lines points">${lines}${expLine}</div>`;
}

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

// Phiên bản hiện tại hiển thị trong thẻ Cập nhật. App thật lấy đúng version của
// bản build qua getVersion(); trình duyệt thường/demo thì giữ chữ v0.1.8 ghi sẵn.
let appVersionText = "v0.1.18";
async function showAppVersion() {
  const el = $("appVersion");
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    appVersionText = "v" + (await getVersion());
  } catch {}
  if (el) el.textContent = appVersionText;
}
showAppVersion();

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
      st.textContent = `Đang dùng bản mới nhất (${appVersionText}).`;
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
    { type: "autoclaw", user_id: "autoclaw:demo", username: "", email: "demo-autoclaw@example.com", device_mid: "", jwt: "", zai_business_token: "", autoclaw_token: "demo" },
  ];
  currentUserId = accountsCache[0].user_id;
  pointsCache.set("autoclaw:demo", {
    available: 125000, unit: "Điểm",
    wallets: [
      { bucket_id: "daily", show_name: "Điểm tặng", remaining_points: 5000, balance_view: "5000" },
      { bucket_id: "monthly", show_name: "Điểm gói", remaining_points: 120000, balance_view: "120000" },
    ],
    subscription: { planName: "AutoClaw Pro", status: "active" },
    expiring: { points: 5000, text: "5000 điểm" },
    raw: {}, user_id: "demo", email: "demo-autoclaw@example.com",
  });
  usageCache.set(accountsCache[0].user_id, {
    total_remaining: 422000000, total_used: 86000000, total_granted: 508000000, has_quota: true,
    balances: [
      { show_name: "GLM-5.3", meter: "tokens", unit_type: "tokens", total_units: 3000000, used_units: 0, remaining_units: 3000000, period_end: now + 86400 * 7, expires_at: now + 86400 * 30 },
      { show_name: "GLM-5.3-Flash", meter: "tokens", unit_type: "tokens", total_units: 5000000, used_units: 0, remaining_units: 5000000, period_end: now + 86400 * 7, expires_at: now + 86400 * 30 },
      { show_name: "ZCODE WEEKEND BUILD", meter: "tokens", unit_type: "tokens", total_units: 500000000, used_units: 86000000, remaining_units: 414000000, period_end: now + 86400 * 2, expires_at: now + 86400 * 2 },
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
// Nạp tuần tự (await): loadAccounts() ghi đè toàn bộ accountsCache, nên
// phải đợi nó xong rồi mới nạp dòng AutoClaw — nạp song song thì
// list_accounts về sau sẽ xóa mất các dòng AutoClaw vừa thêm.
isTauri().then(async () => {
  if (new URLSearchParams(location.search).has("demo")) { seedDemo(); return; }
  await loadAccounts();
  await loadAutoclawAccounts();
  // Auto-check update khi mở app (silent, không popup dialog — dialog:false trong tauri.conf.json)
  checkForUpdates(false);
});
