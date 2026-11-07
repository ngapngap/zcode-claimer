import { invoke } from "@tauri-apps/api/core";

const $ = (id) => document.getElementById(id);
let selectedPlanId = null;
let captchaVerifyParam = "";
let captchaRegion = "";

$("genMid").onclick = async () => {
  $("deviceMid").value = await invoke("generate_device_mid");
};

$("btnPreview").onclick = async () => {
  const jwt = $("jwt").value.trim();
  let deviceMid = $("deviceMid").value.trim();
  if (!deviceMid) { deviceMid = await invoke("generate_device_mid"); $("deviceMid").value = deviceMid; }
  $("previewResult").textContent = "Đang báo activation + preview...";
  try {
    await invoke("report_activation", { jwt, deviceMid });
    const result = await invoke("preview_plans", { jwt, deviceMid });
    if (result?.NotDeployed) {
      $("previewResult").textContent = "活动还没上线 (404) — chờ mở bán";
      return;
    }
    const plans = result?.Plans || [];
    if (!plans.length) {
      $("previewResult").textContent = "Không có套餐 nào hôm nay";
      return;
    }
    $("step-claim").style.display = "block";
    $("plansList").innerHTML = plans.map(p =>
      `<div class="plan-item" data-id="${p.plan_id}"><span><b>${p.name}</b> — ${p.plan_id}</span><span>${p.entitlements?.[0]?.grant_units || 0} ${p.entitlements?.[0]?.unit_type || ""}</span></div>`
    ).join("");
    document.querySelectorAll(".plan-item").forEach(el => {
      el.onclick = () => {
        document.querySelectorAll(".plan-item").forEach(x => x.classList.remove("selected"));
        el.classList.add("selected");
        selectedPlanId = el.dataset.id;
        $("btnCaptcha").disabled = false;
      };
    });
    $("previewResult").textContent = `Tìm thấy ${plans.length}套餐`;
  } catch (e) {
    $("previewResult").textContent = "Lỗi: " + e;
  }
};

$("btnCaptcha").onclick = async () => {
  // Lấy captcha config
  let config;
  try { config = await invoke("get_captcha_config"); } catch (e) { $("claimResult").textContent = "Lấy captcha config lỗi: " + e; return; }
  if (!config) { $("claimResult").textContent = "上游没有下发 captcha 配置 — không cần captcha, bấm Lãnh luôn"; $("btnClaim").disabled = false; return; }
  if (!config.enabled) { $("claimResult").textContent = "上游说不需要验证码 — bấm Lãnh luôn"; $("btnClaim").disabled = false; return; }

  // Khởi tạo Aliyun SDK (copy nguyên từ Agent2API ui/aliyun-captcha.js logic)
  window.AliyunCaptchaConfig = { region: config.region, prefix: config.prefix };
  const script = document.createElement("script");
  script.src = "https://o.alicdn.com/captcha-frontend/aliyunCaptcha/AliyunCaptcha.js";
  document.head.appendChild(script);
  await new Promise(r => setTimeout(r, 2000)); // warmup

  window.initAliyunCaptcha({
    SceneId: config.scene_id,
    mode: "popup",
    element: "#captcha-container",
    button: "#captcha-button",
    captchaVerifyCallback: (param) => {
      captchaVerifyParam = typeof param === "string" ? param : JSON.stringify(param);
      captchaRegion = config.region;
      $("claimResult").textContent = "Captcha OK — bấm Lãnh套餐";
      $("btnClaim").disabled = false;
      return { captchaResult: true, bizResult: true };
    },
    getInstance: (inst) => { window._captchaInst = inst; },
    slideStyle: { width: 360, height: 40 },
    language: "zh-cn",
  });
  // Click invisible button để bật slider
  $("captcha-button").click();
  $("claimResult").textContent = "Kéo slider để hoàn thành captcha...";
};

$("btnClaim").onclick = async () => {
  if (!selectedPlanId) { $("claimResult").textContent = "Chưa chọn套餐"; return; }
  const jwt = $("jwt").value.trim();
  const deviceMid = $("deviceMid").value.trim();
  $("claimResult").textContent = "Đang claim...";
  try {
    const result = await invoke("claim_plan", {
      jwt, deviceMid,
      planId: selectedPlanId,
      captchaVerifyParam,
      captchaRegion,
    });
    if (result?.Claimed) {
      $("claimResult").textContent = "✅ Đã lãnh: " + result.Claimed.plan_id;
      $("step-export").style.display = "block";
    } else if (result?.Failed) {
      $("claimResult").textContent = `❌ ${result.Failed.failure}: ${result.Failed.message} (code ${result.Failed.code})`;
    }
  } catch (e) {
    $("claimResult").textContent = "Lỗi: " + e;
  }
};

$("btnExport").onclick = async () => {
  const jwt = $("jwt").value.trim();
  const json = await invoke("export_router_json", {
    jwt,
    userId: $("userId").value,
    username: $("username").value,
    email: $("email").value,
    zaiBusinessToken: $("bizToken").value,
  });
  $("exportJson").textContent = json;
  $("btnDownload").onclick = () => {
    const blob = new Blob([json], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "zcode-export.json";
    a.click();
  };
};
