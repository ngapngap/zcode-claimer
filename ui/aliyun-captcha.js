/* ZCode Claimer · Bo giai captcha Aliyun (slider) — NGUYEN LOGIC, CHUOI TIENG VIET
 *
 * ── Tai sao tach rieng thanh file ─────────────────────────────
 * Ma nay di kem theo app claim ZCode. Flow claim can vuot qua captcha
 * Aliyun (API nhan goi can header `X-Aliyun-Captcha-Verify-Param`
 * de xac dinh co can hien slider hay khong qua cau hinh upstream
 * `enabled` — khi `enabled: false` frontend KHONG duoc hien slider).
 * Luong: giai captcha -> lay verifyParam -> claim plan.
 *
 * ── Hop dong goi: `request` do caller truyen vao ─────────────
 * `solve(config, request)` goi `request(verifyParam)` khi co chuoi xac thuc
 * de caller xu ly viec cua no, tra ve `{captchaResult, bizResult}` bao SDK
 * qua / chua qua (ca hai true thi SDK moi dong slider). Module nay khong
 * biet gi ve "goi" hay "tai khoan" — chi biet captcha.
 *
 * ── Khong dung crate ngoai / khong doi Constants ─────────────
 * Giua nguyen cau truc logic cua ban goc (chi dich comment + loi sang TIENG VIET).
 * SDK Aliyun load tu `o.alicdn.com` (`AliyunCaptcha.js`). Tauri CSP = null
 * (xem tauri.conf.json) nen script ngoai load duoc. Neu khoa CSP phai allow
 * `https://o.alicdn.com` + `https://*.alicdn.com`.
 *
 * ── Chi mot luong captcha tai mot thoi diem ──────────────────
 * Trang thai module-level (SDK client cung vay): hai caller goi dong thoi
 * se da cham nhau. Giao dien chi mo mot luong tai mot thoi diem nen khong
 * can hang doi, `solve` dung so the he (generation) de vo hieu luong truoc
 * (callback den tre thi bi huy khi the he khong khop).
 */

(() => {
  // ── Hang so: giua nguyen nhu ban goc ─────────────────────────
  const ALIYUN_CAPTCHA_SCRIPT_URL =
    'https://o.alicdn.com/captcha-frontend/aliyunCaptcha/AliyunCaptcha.js';
  const SCRIPT_ID = 'aliyun-captcha-sdk';
  /** Diem gan SDK (container cua slider) */
  const ELEMENT_ID = 'aliyun-captcha-element';
  /** Nut kich hoat: SDK yeu cau truyen 1 button selector, click no moi hien slider.
   *  Lam 1x1 trong suot khong thay, code `button.click()` de kich hoat. */
  const BUTTON_ID = 'aliyun-captcha-trigger';

  const SCRIPT_LOAD_TIMEOUT_MS = 40000;
  const INIT_TIMEOUT_MS = 40000;
  const VERIFY_TIMEOUT_MS = 120000;
  /** Timeout cho mintTraceless (bi treo thi cast lai, khong cho doi) */
  const MINT_TIMEOUT_MS = 20000;
  /** Sau khi init phai doi it nhat 2.1 giay moi click nut (SDK chua warmup thi click vo hieu) */
  const MINIMUM_WARMUP_MS = 2100;
  /** Ket qua init toi da tai su dung 19 phut (qua thi tao lai) */
  const INITIALIZATION_MAX_AGE_MS = 19 * 60000;

  // ── Trang thai module-level (mot captcha flow tai mot thoi diem) ──
  let scriptLoadPromise = null;
  let initializationPromise = null;
  /** Khoa init (region:prefix:sceneId:language) — cau hinh doi thi tao lai instance */
  let initializationKey = '';
  /** So the he: khi async quay lai thi dung no de biet "luong nay co bi vo hieu khong" */
  let initializationGeneration = 0;
  let initializedAt = 0;
  let captchaInstance = null;
  /** Luong dang cho ket qua captcha */
  let pendingVerification = null;

  const sleep = ms => new Promise(resolve => window.setTimeout(resolve, ms));

  /**
   * Map ngon ngu (tuong duong `resolveAliyunCaptchaLanguage`).
   * Aliyun chi nhan cac ma ngan nay, truyen ma la thi no coi nhu khong hieu
   * va roi xuong tieng Anh — nen phai chuan hoa o day, khong duoc truyen
   * thang `vi-VN` v.v. cho SDK.
   */
  function resolveAliyunCaptchaLanguage(language) {
    const normalized = String(language || '').trim().replace(/_/g, '-').toLowerCase();
    return normalized.startsWith('vi') ? 'vi' : 'en';
  }

  /** Ngon ngu hien tai cua giao dien (lay tu <html lang>, mac dinh tieng Viet) */
  const currentLanguage = () => document.documentElement.lang || 'vi';

  function getInitAliyunCaptcha() {
    const value = window.initAliyunCaptcha;
    return typeof value === 'function' ? value : null;
  }

  /** Loi khi flow captcha that bai (co cau bao cho user) */
  class CaptchaError extends Error {
    constructor(message) {
      super(message);
      this.name = 'CaptchaError';
    }
  }

  /** User tu huy (nhan "Huy" hoac dong popup) — khong bao do */
  class CaptchaCancelledError extends CaptchaError {
    constructor() {
      super('Da huy captcha');
      this.name = 'CaptchaCancelledError';
    }
  }

  function removeCaptchaElements() {
    document.getElementById(ELEMENT_ID)?.remove();
    document.getElementById(BUTTON_ID)?.remove();
  }

  /** Vo hieu instance hien tai (cau hinh doi / captcha dung xong can tao lai) */
  function invalidateInitialization() {
    const instance = captchaInstance;
    initializationGeneration += 1;
    captchaInstance = null;
    initializationPromise = null;
    initializedAt = 0;
    removeCaptchaElements();
    try { instance?.destroy?.(); } catch { /* bo qua: instance da khong dung duoc */ }
  }

  /**
   * Chuan bi container slider + nut kich hoat.
   * Ca hai element deu do SDK tim bang id: `element` la diem roi cua slider,
   * `button` la trigger hien slider. Nut lam 1x1 trong suot, pointer-events none
   * nen user khong thay / khong click duoc — chi code `button.click()` mot lan
   * sau khi san sang, tranh "user tu click ra hai slider".
   */
  function ensureCaptchaElements() {
    let element = document.getElementById(ELEMENT_ID);
    if (!element) {
      element = document.createElement('div');
      element.id = ELEMENT_ID;
      element.style.position = 'relative';
      element.style.zIndex = '2147483000';
      document.body.appendChild(element);
    }
    let button = document.getElementById(BUTTON_ID);
    if (!button) {
      button = document.createElement('button');
      button.id = BUTTON_ID;
      button.type = 'button';
      button.tabIndex = -1;
      button.setAttribute('aria-hidden', 'true');
      button.style.position = 'fixed';
      button.style.width = '1px';
      button.style.height = '1px';
      button.style.opacity = '0';
      button.style.pointerEvents = 'none';
      button.style.overflow = 'hidden';
      document.body.appendChild(button);
    }
    return button;
  }

  /**
   * Load script SDK.
   * `window.AliyunCaptchaConfig` PHẢI set TRUOC khi script load — SDK doc no
   * de biet dung site cua Aliyun nao (`region` / `prefix`). Set sau thi SDK dung
   * site mac dinh va bieu hien "captcha hien ra nhung quay vo han".
   */
  function loadAliyunCaptchaScript(config) {
    window.AliyunCaptchaConfig = { region: config.region, prefix: config.prefix };
    if (getInitAliyunCaptcha()) return Promise.resolve();
    if (scriptLoadPromise) return scriptLoadPromise;
    scriptLoadPromise = new Promise((resolve, reject) => {
      const existing = document.getElementById(SCRIPT_ID);
      const script = existing || document.createElement('script');
      let timer = 0;
      const cleanup = () => {
        window.clearTimeout(timer);
        script.removeEventListener('load', onLoad);
        script.removeEventListener('error', onError);
      };
      const fail = error => {
        cleanup();
        scriptLoadPromise = null;
        script.remove();
        reject(error);
      };
      const onLoad = () => {
        cleanup();
        if (getInitAliyunCaptcha()) resolve();
        else fail(new CaptchaError('Thanh phan captcha load bat thuong, thu lai'));
      };
      const onError = () => fail(new CaptchaError('Load thanh phan captcha that bai, kiem tra mang roi thu lai'));
      timer = window.setTimeout(
        () => fail(new CaptchaError('Load thanh phan captcha qua thoi gian, kiem tra mang roi thu lai')),
        SCRIPT_LOAD_TIMEOUT_MS,
      );
      script.addEventListener('load', onLoad);
      script.addEventListener('error', onError);
      if (!existing) {
        script.id = SCRIPT_ID;
        script.async = true;
        script.src = ALIYUN_CAPTCHA_SCRIPT_URL;
        document.head.appendChild(script);
      }
    });
    return scriptLoadPromise;
  }

  /** Ket thuc mot lan cho doi */
  function settlePending(pending, outcome) {
    if (pendingVerification !== pending || pending.settled) return false;
    pending.settled = true;
    window.clearTimeout(pending.timer);
    if (outcome.error) pending.reject(outcome.error);
    else pending.resolve(outcome.value);
    return true;
  }

  /**
   * Callback cua SDK: nhan chuoi xac thuc khong minh (opaque) -> giao cho caller.
   * Tra ve phai la `{captchaResult, bizResult}`: SDK dung no de quyet "qua chua".
   */
  async function captchaVerifyCallback(generation, captchaVerifyParam) {
    const pending = pendingVerification;
    if (!pending || pending.generation !== generation) {
      return { captchaResult: false, bizResult: false };
    }
    if (typeof captchaVerifyParam !== 'string' || captchaVerifyParam.length === 0) {
      settlePending(pending, {
        error: new CaptchaError('Xac thuc captcha that bai, thu lai'),
      });
      return { captchaResult: false, bizResult: false };
    }
    try {
      const result = await pending.request(captchaVerifyParam);
      if (pendingVerification !== pending || pending.settled) {
        return {
          captchaResult: Boolean(result?.captchaResult),
          bizResult: Boolean(result?.bizResult),
        };
      }
      settlePending(pending, { value: result });
      return {
        captchaResult: Boolean(result?.captchaResult),
        bizResult: Boolean(result?.bizResult),
      };
    } catch (error) {
      settlePending(pending, { error });
      return { captchaResult: false, bizResult: false };
    }
  }

  /**
   * Khoi tao instance SDK.
   * Cung cau hinh thi chi init mot lan, tai su dung trong 19 phut; cau hinh doi
   * hoac het han thi tao lai (xem `invalidateInitialization`).
   */
  function initializeAliyunCaptcha(config) {
    const language = resolveAliyunCaptchaLanguage(currentLanguage());
    const key = `${config.region}:${config.prefix}:${config.sceneId}:${language}`;
    if (initializationPromise && initializationKey === key) {
      const inFlight = initializedAt === 0;
      const fresh = Date.now() - initializedAt < INITIALIZATION_MAX_AGE_MS;
      if (inFlight || fresh) return initializationPromise;
      invalidateInitialization();
    }
    if (initializationKey && initializationKey !== key) invalidateInitialization();
    initializationKey = key;
    const generation = ++initializationGeneration;
    initializationPromise = (async () => {
      await loadAliyunCaptchaScript(config);
      ensureCaptchaElements();
      const initAliyunCaptcha = getInitAliyunCaptcha();
      if (!initAliyunCaptcha) throw new CaptchaError('Thanh phan captcha khong dung duoc, thu lai');
      await new Promise((resolve, reject) => {
        let settled = false;
        let boundInstance = null;
        const timer = window.setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new CaptchaError('Khoi tao thanh phan captcha qua thoi gian, thu lai'));
        }, INIT_TIMEOUT_MS);
        const settle = callback => {
          if (settled) return false;
          settled = true;
          window.clearTimeout(timer);
          callback();
          return true;
        };
        try {
          initAliyunCaptcha({
            SceneId: config.sceneId,
            mode: 'popup',
            element: `#${ELEMENT_ID}`,
            button: `#${BUTTON_ID}`,
            captchaVerifyCallback: param => captchaVerifyCallback(generation, param),
            onBizResultCallback: () => {},
            getInstance: instance => {
              boundInstance = instance;
              if (generation !== initializationGeneration) {
                try { instance.destroy?.(); } catch { /* bo qua */ }
                return;
              }
              if (!settle(() => {
                captchaInstance = instance;
                initializedAt = Date.now();
                resolve();
              })) {
                try { instance.destroy?.(); } catch { /* bo qua */ }
              }
            },
            slideStyle: { width: 360, height: 40 },
            language,
            onError: error => {
              const pending = pendingVerification;
              if (pending && pending.generation === generation && pending.instance === boundInstance) {
                settlePending(pending, {
                  error: new CaptchaError(
                    (error && error.message) || 'Xac thuc captcha that bai, thu lai',
                  ),
                });
                return;
              }
              if (settled) {
                if (generation === initializationGeneration
                  && (!boundInstance || captchaInstance === boundInstance)) {
                  invalidateInitialization();
                }
                return;
              }
              settle(() => reject(new CaptchaError(
                (error && error.message) || 'Thanh phan captcha khong dung duoc, thu lai',
              )));
            },
          });
        } catch (error) {
          settle(() => reject(new CaptchaError(
            (error && error.message) || 'Thanh phan captcha khong dung duoc, thu lai',
          )));
        }
      });
      if (!captchaInstance) throw new CaptchaError('Thanh phan captcha khong dung duoc, thu lai');
    })();
    return initializationPromise;
  }

  /** Cho SDK warmup xong (click qua nhanh thi slider khong hien) */
  async function waitForWarmup() {
    const remaining = MINIMUM_WARMUP_MS - (Date.now() - initializedAt);
    if (remaining > 0) await sleep(remaining);
  }

  /**
   * Chay mot luot captcha day du: init -> warmup -> hien slider -> user keo -> lay chuoi giao caller.
   *
   * `config`: `{ region, prefix, sceneId }` (lay tu cau hinh upstream).
   * `request(verifyParam)`: caller truyen vao de dung chuoi do, tra
   * `{captchaResult, bizResult}`. Gia tri no resolve ra se tro thanh gia tri tra
   * ve nguyen ven cua `solve()` — caller co the dung no de dem ket qua ve.
   *
   * Timeout 120 giay chi bao "keo slider + mot request do". Caller neu lam
   * luong dai trong `request` (cho callback dang nhap...) se bi timeout nay
   * giet nham — luong dai phai dat SAU khi `solve()` return.
   */
  async function solve(config, request) {
    await initializeAliyunCaptcha(config);
    await waitForWarmup();
    const button = ensureCaptchaElements();
    const instance = captchaInstance;
    if (!instance) throw new CaptchaError('Thanh phan captcha khong dung duoc, thu lai');
    const generation = initializationGeneration;
    return new Promise((resolve, reject) => {
      const pending = {
        generation,
        instance,
        request,
        resolve,
        reject,
        timer: 0,
        settled: false,
      };
      pending.timer = window.setTimeout(() => {
        settlePending(pending, {
          error: new CaptchaError('Xac thuc captcha qua thoi gian, thu lai'),
        });
      }, VERIFY_TIMEOUT_MS);
      pendingVerification = pending;
      button.click();
    });
  }

  /* ─── Cast khong dau (mintTraceless) — GIU NGUYEN LOGIC, chi dich comment ─── */
  let mintSeq = 0;
  const mintIds = () => {
    mintSeq += 1;
    return {
      elementId: `zcode-mint-captcha-element-${mintSeq}`,
      buttonId: `zcode-mint-captcha-trigger-${mintSeq}`,
    };
  };

  let mintInstancePromise = null;
  let mintInstanceKey = '';
  let mintInstanceRef = null;
  let mintElementId = '';
  let mintButtonId = '';
  let mintWaiter = null;
  let mintGeneration = 0;
  let mintInitializedAt = 0;

  function simulateHumanActivity() {
    try {
      if (document.hidden) {
        Object.defineProperty(document, 'hidden', { value: false, configurable: true });
      }
      if (document.visibilityState !== 'visible') {
        Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
      }
    } catch { /* bo qua */ }

    try {
      for (let i = 0; i < 3; i++) {
        const x = Math.floor(100 + Math.random() * 300);
        const y = Math.floor(100 + Math.random() * 200);
        const event = new MouseEvent('mousemove', {
          bubbles: true,
          cancelable: true,
          view: window,
          clientX: x,
          clientY: y,
          screenX: x + 50,
          screenY: y + 50,
        });
        window.dispatchEvent(event);
      }
    } catch { /* bo qua */ }
  }

  function ensureMintElements(elementId, buttonId) {
    let element = document.getElementById(elementId);
    if (!element) {
      element = document.createElement('div');
      element.id = elementId;
      element.style.position = 'fixed';
      element.style.left = '-9999px';
      element.style.top = '0';
      element.style.width = '360px';
      element.style.height = '40px';
      document.body.appendChild(element);
    }
    let button = document.getElementById(buttonId);
    if (!button) {
      button = document.createElement('button');
      button.id = buttonId;
      button.type = 'button';
      button.tabIndex = -1;
      button.setAttribute('aria-hidden', 'true');
      button.style.position = 'fixed';
      button.style.left = '-9999px';
      button.style.width = '1px';
      button.style.height = '1px';
      button.style.opacity = '0';
      button.style.pointerEvents = 'none';
      document.body.appendChild(button);
    }
  }

  function invalidateMintInstance() {
    const instance = mintInstanceRef;
    mintInstancePromise = null;
    mintInstanceKey = '';
    mintInstanceRef = null;
    mintInitializedAt = 0;
    try { instance?.destroy?.(); } catch { /* bo qua */ }
    try { document.getElementById(mintElementId)?.remove(); } catch { /* bo qua */ }
    try { document.getElementById(mintButtonId)?.remove(); } catch { /* bo qua */ }
    mintElementId = '';
    mintButtonId = '';
  }

  function failMint(error) {
    const waiter = mintWaiter;
    mintWaiter = null;
    invalidateMintInstance();
    if (waiter) waiter.reject(error);
  }

  function validateVerifyParam(value) {
    if (typeof value !== 'string' || value.trim().length < 200) {
      throw new CaptchaError('Chuoi xac thuc tra ve khong day du (thu lai)');
    }
    const text = value.trim();
    try {
      const json = JSON.parse(atob(text));
      const token = json && (json.securityToken || json.SecurityToken);
      if (!token || String(token).length < 50) throw new Error('no securityToken');
    } catch {
      throw new CaptchaError('Chuoi xac thuc khong dung dinh dang upstream nhan (thu lai)');
    }
    return text;
  }

  function deliverMintResult(result) {
    const waiter = mintWaiter;
    if (!waiter) return;
    let value = result;
    if (result && typeof result === 'object') {
      value = result.verifyParam || result.captchaVerifyParam || result.data || result.param;
    }
    let param = null;
    let failure = null;
    try {
      param = validateVerifyParam(value);
    } catch (error) {
      failure = error;
    }
    mintWaiter = null;
    if (failure) {
      failMint(failure);
      return;
    }
    invalidateMintInstance();
    waiter.resolve(param);
  }

  async function mintTraceless(config) {
    const instance = await ensureMintInstance(config);
    const elapsed = Date.now() - mintInitializedAt;
    if (elapsed < MINIMUM_WARMUP_MS) {
      await sleep(MINIMUM_WARMUP_MS - elapsed);
    }
    simulateHumanActivity();

    const generation = ++mintGeneration;
    return new Promise((resolve, reject) => {
      const timer = window.setTimeout(() => {
        if (!mintWaiter || mintWaiter.generation !== generation) return;
        mintWaiter = null;
        invalidateMintInstance();
        reject(new CaptchaError('Cast khong dau qua thoi gian, thu lai'));
      }, MINT_TIMEOUT_MS);
      mintWaiter = {
        generation,
        resolve: value => { window.clearTimeout(timer); resolve(value); },
        reject: error => { window.clearTimeout(timer); reject(error); },
      };
      const start = typeof instance.startTracelessVerification === 'function'
        ? instance.startTracelessVerification
        : instance.show;
      if (typeof start !== 'function') {
        failMint(new CaptchaError('Thanh phan captcha khong ho tro cast khong dau'));
        return;
      }
      try {
        start.call(instance);
      } catch (error) {
        failMint(new CaptchaError((error && error.message) || 'Khai dong thanh phan captcha that bai'));
      }
    });
  }

  function ensureMintInstance(config) {
    const language = resolveAliyunCaptchaLanguage(currentLanguage());
    const key = config.region + ':' + config.prefix + ':' + config.sceneId + ':' + language;
    if (mintInstancePromise && mintInstanceKey === key) return mintInstancePromise;
    if (mintInstanceRef) invalidateMintInstance();
    mintInstanceKey = key;
    const pending = (async () => {
      await loadAliyunCaptchaScript(config);
      const ids = mintIds();
      mintElementId = ids.elementId;
      mintButtonId = ids.buttonId;
      ensureMintElements(ids.elementId, ids.buttonId);
      const initAliyunCaptcha = getInitAliyunCaptcha();
      if (!initAliyunCaptcha) throw new CaptchaError('Thanh phan captcha khong dung duoc, thu lai');
      return new Promise((resolve, reject) => {
        let settled = false;
        const timer = window.setTimeout(() => {
          if (settled) return;
          settled = true;
          mintInstancePromise = null;
          reject(new CaptchaError('Khoi tao thanh phan captcha qua thoi gian, thu lai'));
        }, INIT_TIMEOUT_MS);
        try {
          initAliyunCaptcha({
            SceneId: config.sceneId,
            mode: 'popup',
            region: config.region,
            prefix: config.prefix,
            element: '#' + ids.elementId,
            button: '#' + ids.buttonId,
            captchaLogoImg: '',
            showErrorTip: false,
            language,
            getInstance: instance => {
              if (settled) return;
              settled = true;
              window.clearTimeout(timer);
              mintInstanceRef = instance;
              mintInitializedAt = Date.now();
              resolve(instance);
            },
            success: result => deliverMintResult(result),
            fail: error => failMint(new CaptchaError(
              (error && error.message) || 'Xac thuc captcha that bai, thu lai',
            )),
            onError: error => failMint(new CaptchaError(
              (error && error.message) || 'Thanh phan captcha loi, thu lai',
            )),
          });
        } catch (error) {
          if (settled) return;
          settled = true;
          window.clearTimeout(timer);
          mintInstancePromise = null;
          reject(new CaptchaError((error && error.message) || 'Khoi tao thanh phan captcha that bai'));
        }
      });
    })();
    pending.catch(() => {
      mintInstancePromise = null;
      mintInstanceKey = '';
    });
    mintInstancePromise = pending;
    return pending;
  }

  function cancel() {
    const pending = pendingVerification;
    if (!pending) return false;
    const cancelled = settlePending(pending, { error: new CaptchaCancelledError() });
    if (cancelled) invalidateInitialization();
    return cancelled;
  }

  const isBusy = () => pendingVerification !== null;

  window.wbAliyunCaptcha = {
    solve,
    cancel,
    isBusy,
    mintTraceless,
    resetMint: invalidateMintInstance,
    CaptchaError,
    CaptchaCancelledError,
  };
})();
