import { invoke } from "@tauri-apps/api/core";

// ── UI claim ZCode: 4 tính năng đúng theo phạm vi hiện tại ──
// 1) nhập provider (JWT/deviceMid/userId/username/email/zaiBusinessToken)
// 2) claim (preview → captcha slider → claim plan) qua ui/aliyun-captcha.js solve()
// 3) check tài khoản (query_balance / refreshQuota — quota, plans, balances)
// 4) xuất định dạng router (export_router_json)
// NGOÀI PHẠM VI (không làm): scheduler 18:00 VN, tray notification,
// traceless pool background (ui/zcode-captcha-pool.js), claim history UI,
// Docker/headless. Rust history (persistence.rs) vẫn giữ ở backend.

const $ = (id) => document.getElementById(id);
let selectedPlanId = null;
let captchaVerifyParam = "";
let captchaRegion = "";
let jitterTimer = null;

$("genMid").onclick = async () => {
  $("deviceMid").value = await invoke("generate_device_mid");
  await saveCfg();
};

function fmtUnits(n) {
  const v = Number(n) || 0;
  if (v >= 1e8) return (v / 1e8).toFixed(v % 1e8 === 0 ? 0 : 1) + " triệu";
  if (v >= 1e6) return (v / 1e6).toFixed(v % 1e6 === 0 ? 0 : 1) + "M";
  if (v >= 1e3) return (v / 1e3).toFixed(1) + "K";
  return String(v);
}
function fmtTime(sec) {
  if (!sec) return "";
  try { return new Date(sec * 1000).toLocaleString("vi-VN"); } catch { return ""; }
}
function stateTag(s) {
  const map = { active: "Đang hiệu lực", pending: "Chờ hiệu lực", expired: "Đã hết hạn", unknown: "Không rõ" };
  return `<span class="tag ${s}">${map[s] || s}</span>`;
}
async function ensureMid() {
  if (!$("deviceMid").value.trim()) $("deviceMid").value = await invoke("generate_device_mid");
  return $("deviceMid").value.trim();
}

// ── Persistence Rust (t1) thay localStorage JWT ──
// Ưu tiên load_saved_config → save_saved_config. Chỉ khi Tauri không sẵn sàng
// (mở bằng trình duyệt thuần cho dev) mới fallback localStorage — trade-off này
// được ghi chú trong docs/security.md (bản release nên dùng Rust persistence).
function readCfg() {
  return {
    device_mid: $("deviceMid").value.trim() || "",
    jwt: $("jwt").value.trim() || "",
    user_id: $("userId").value.trim() || "",
    username: $("username").value.trim() || "",
    email: $("email").value.trim() || "",
    zai_business_token: $("bizToken").value.trim() || "",
  };
}
function writeCfg(cfg) {
  $("deviceMid").value = cfg.device_mid || "";
  $("jwt").value = cfg.jwt || "";
  $("userId").value = cfg.user_id || "";
  $("username").value = cfg.username || "";
  $("email").value = cfg.email || "";
  $("bizToken").value = cfg.zai_business_token || "";
}
let tauriReady = null;
async function isTauri() {
  if (!tauriReady) {
    tauriReady = invoke("load_saved_config")
      .then(cfg => { writeCfg(cfg); return true; })
      .catch(() => {
        // Fallback dev (trình duyệt thuần): localStorage — không giữ cho release.
        try {
          $("jwt").value = localStorage.getItem("zcode-claimer-jwt") || "";
          $("deviceMid").value = localStorage.getItem("zcode-claimer-mid") || "";
        } catch {}
        return false;
      });
  }
  return tauriReady;
}
async function saveCfg() {
  const cfg = readCfg();
  try {
    await invoke("save_saved_config", { config: cfg });
  } catch {
    try {
      localStorage.setItem("zcode-claimer-jwt", $("jwt").value);
      localStorage.setItem("zcode-claimer-mid", $("deviceMid").value);
    } catch {}
  }
}
["jwt", "deviceMid", "userId", "username", "email", "bizToken"].forEach(id =>
  $(id).addEventListener("change", saveCfg));

function renderQuota(r) {
  const total = r.total_remaining, used = r.total_used, granted = r.total_granted || (total + used);
  const pct = granted > 0 ? Math.min(100, Math.round(used / granted * 100)) : 0;
  $("quotaSummary").innerHTML =
    `<div class="quota-total"><div><div class="label">Còn lại</div><div class="big">${fmtUnits(total)} tokens</div></div><div style="text-align:right"><div class="label">Đã dùng ${fmtUnits(used)} / ${fmtUnits(granted)}</div><div class="bar" style="width:200px"><div class="bar-fill ${pct > 90 ? 'low' : ''}" style="width:${pct}%"></div></div><div class="meta" style="font-size:12px;color:#6e6e73">${pct}%</div></div></div>`;
  // buckets
  if (r.balances?.length) {
    $("quotaBuckets").innerHTML = '<div class="section-title">Các bucket số dư</div>' + r.balances.map(b => {
      const p = b.total_units > 0 ? Math.round(b.used_units / b.total_units * 100) : 0;
      return `<div class="bucket"><div class="bucket-row"><span class="name">${b.show_name || b.meter || b.bucket_id}</span><span>${fmtUnits(b.remaining_units)} / ${fmtUnits(b.total_units)} ${b.unit_type || ""}</span></div><div class="bar"><div class="bar-fill ${p > 90 ? 'low' : ''}" style="width:${p}%"></div></div><div class="meta">${b.period_end ? "Hết hạn " + fmtTime(b.period_end) : ""} ${b.expires_at ? "· Hết hạn " + fmtTime(b.expires_at) : ""}</div></div>`;
    }).join("");
  } else $("quotaBuckets").innerHTML = '<div class="quota-empty">Chưa có bucket số dư (có thể quota nằm ở kênh gói hoạt động)</div>';
  // plans
  if (r.plans?.length) {
    $("quotaPlans").innerHTML = '<div class="section-title">Chi tiết gói</div>' + r.plans.map(p =>
      `<div class="plan-row"><span><b>${p.name || p.plan_id}</b> <span style="color:#6e6e73;font-size:12px">${p.plan_id}</span></span><span style="display:flex;gap:8px;align-items:center">${stateTag(p.status)}<span style="font-size:12px;color:#6e6e73">${p.ends_at ? "Đến " + fmtTime(p.ends_at) : ""}</span></span></div>`
    ).join("");
  } else $("quotaPlans").innerHTML = "";
}

async function refreshQuota() {
  const jwt = $("jwt").value.trim();
  if (!jwt) { $("quotaSummary").innerHTML = '<div class="quota-empty">Nhập JWT rồi bấm Làm mới (JWT lấy từ ZCode app sau khi login)</div>'; return; }
  $("quotaSummary").textContent = "Đang tải quota...";
  try {
    const mid = await ensureMid();
    renderQuota(await invoke("query_balance", { jwt, deviceMid: mid }));
  } catch (e) {
    $("quotaSummary").innerHTML = `<div style="color:#a50e0e;font-size:13px">Lỗi quota: ${e}</div>`;
  }
}
$("btnRefreshQuota").onclick = refreshQuota;

// Demo dữ liệu mẫu cho quota card (chỉ khi mở với ?demo=1, không cần JWT thật).
if (new URLSearchParams(location.search).has("demo")) {
  const now = Math.floor(Date.now() / 1000);
  renderQuota({
    total_remaining: 4200000, total_used: 800000, total_granted: 5000000,
    balances: [
      { show_name: "Gói Start Plan", meter: "tokens", unit_type: "tokens", total_units: 3000000, used_units: 500000, remaining_units: 2500000, period_end: now + 86400 * 7, expires_at: now + 86400 * 30 },
      { show_name: "Gói Bonus", meter: "tokens", unit_type: "tokens", total_units: 2000000, used_units: 300000, remaining_units: 1700000, period_end: now + 86400 * 3, expires_at: now + 86400 * 10 },
    ],
    plans: [
      { plan_id: "zcode-v3-start-plan-trust-0928", name: "Start Plan", status: "active", ends_at: now + 86400 * 7 },
      { plan_id: "demo-plan-pending", name: "Gói chờ hiệu lực", status: "pending", ends_at: now + 86400 },
    ],
  });
  showRetryHint(1005, now + 3600);
}

$("btnPreview").onclick = async () => {
  const jwt = $("jwt").value.trim();
  await saveCfg();
  const mid = await ensureMid();
  $("previewResult").textContent = "Đang báo activation + preview...";
  try {
    await invoke("report_activation", { jwt, deviceMid: mid });
    const result = await invoke("preview_plans", { jwt, deviceMid: mid });
    if (result?.NotDeployed) { $("previewResult").textContent = "Hoạt động chưa ra mắt (404) — chờ mở bán"; return; }
    const plans = result?.Plans || [];
    if (!plans.length) { $("previewResult").textContent = "Không có gói nào hôm nay"; return; }
    $("step-claim").style.display = "block";
    $("plansList").innerHTML = plans.map(p =>
      `<div class="plan-item" data-id="${p.plan_id}"><span><b>${p.name}</b> — ${p.plan_id}<br><span style="font-size:12px;color:#6e6e73">${p.description || ""}</span></span><span>${p.entitlements?.[0] ? fmtUnits(p.entitlements[0].grant_units) + " " + p.entitlements[0].unit_type : ""}</span></div>`
    ).join("");
    document.querySelectorAll(".plan-item").forEach(el => {
      el.onclick = () => {
        document.querySelectorAll(".plan-item").forEach(x => x.classList.remove("selected"));
        el.classList.add("selected");
        selectedPlanId = el.dataset.id;
        $("btnCaptcha").disabled = false;
      };
    });
    $("previewResult").textContent = `Tìm thấy ${plans.length} gói`;
  } catch (e) { $("previewResult").textContent = "Lỗi: " + e; }
};

// ── Captcha: dùng ui/aliyun-captcha.js solve(config, request) ──
// request(verifyParam) lưu chuoi xac thuc + region, trả về {captchaResult, bizResult}
// để SDK biết qua/chưa; solve() sẽ resolve khi slider đóng. Load script tĩnh
// duy nhất ở index.html (không load 2 lần Aliyun SDK).
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
function showRetryHint(code, failureEndsAt) {
  const el = $("retryHint");
  const hint = retryHint(code, failureEndsAt);
  if (!hint) { el.hidden = true; el.textContent = ""; return; }
  el.hidden = false;
  el.className = "retry-hint " + hint.cls;
  el.textContent = hint.text;
}

$("btnCaptcha").onclick = async () => {
  let config;
  try { config = await invoke("get_captcha_config"); } catch (e) { $("claimResult").textContent = "Lấy cấu hình captcha lỗi: " + e; return; }
  if (!config) { $("claimResult").textContent = "Upstream không gửi cấu hình captcha — không cần captcha, bấm Lãnh luôn"; $("btnClaim").disabled = false; return; }
  if (!config.enabled) { $("claimResult").textContent = "Upstream nói không cần mã xác thực — bấm Lãnh luôn"; $("btnClaim").disabled = false; return; }
  $("claimResult").textContent = "Kéo slider để hoàn thành captcha...";
  try {
    const result = await window.wbAliyunCaptcha.solve(
      { region: config.region, prefix: config.prefix, sceneId: config.scene_id },
      async (verifyParam) => {
        captchaVerifyParam = verifyParam;
        captchaRegion = config.region;
        $("claimResult").textContent = "Captcha OK — bấm Lãnh gói";
        $("btnClaim").disabled = false;
        return { captchaResult: true, bizResult: true };
      }
    );
    // solve resolve với giá trị request trả về; captcha đã lưu ở captchaVerifyParam
    if (!result?.captchaResult) { $("claimResult").textContent = "Captcha chưa đạt — hãy thử lại slider"; $("btnClaim").disabled = !captchaVerifyParam; }
  } catch (e) {
    if (e?.name === "CaptchaCancelledError") { $("claimResult").textContent = "Đã hủy captcha"; return; }
    $("claimResult").textContent = "Lỗi captcha: " + (e?.message || e);
  }
};

$("btnClaim").onclick = async () => {
  if (!selectedPlanId) { $("claimResult").textContent = "Chưa chọn gói"; return; }
  $("claimResult").textContent = "Đang claim...";
  try {
    const result = await invoke("claim_plan", {
      jwt: $("jwt").value.trim(), deviceMid: $("deviceMid").value.trim(),
      planId: selectedPlanId, captchaVerifyParam, captchaRegion,
    });
    if (result?.Claimed) {
      $("claimResult").textContent = "✅ Đã lãnh: " + result.Claimed.plan_id;
      $("step-export").style.display = "block";
      // Ghi history vào backend (Rust giữ, UI không hiện lịch sử theo phạm vi rút gọn)
      try { await invoke("record_claim_history", { planId: result.Claimed.plan_id, date: new Date().toISOString().slice(0, 10), outcome: "claimed", code: 0, message: "", failureEndsAt: null }); } catch {}
      refreshQuota();
    } else if (result?.Failed) {
      $("claimResult").textContent = `❌ ${result.Failed.failure}: ${result.Failed.message} (code ${result.Failed.code})`;
      showRetryHint(result.Failed.code, result.Failed.failure_ends_at);
      try { await invoke("record_claim_history", { planId: selectedPlanId, date: new Date().toISOString().slice(0, 10), outcome: "failed", code: result.Failed.code, message: result.Failed.message || "", failureEndsAt: result.Failed.failure_ends_at ?? null }); } catch {}
      if (result.Failed.code === 3007) $("btnClaim").disabled = true;
    }
  } catch (e) { $("claimResult").textContent = "Lỗi: " + e; }
};

$("btnExport").onclick = async () => {
  const json = await invoke("export_router_json", {
    jwt: $("jwt").value.trim(), userId: $("userId").value, username: $("username").value,
    email: $("email").value, zaiBusinessToken: $("bizToken").value,
  });
  $("exportJson").textContent = json;
  $("btnDownload").onclick = () => {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([json], { type: "application/json" }));
    a.download = "zcode-export.json"; a.click();
  };
};

// Khởi tạo: nạp config đã lưu từ Rust persistence khi mở app
isTauri().then(() => {
  if ($("jwt").value.trim()) refreshQuota();
});
