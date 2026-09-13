// QA probe runner for the live app via MCP socket (new build verification)
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const scenario = process.argv[2] || 'crossdevice';

const scenarios = {
  // Full NFC/BLE card inspection on Cross-Device tab
  crossdevice: () => `(()=>{
    const t = document.body.innerText;
    const hasExcuse = /diferent|excuse|not.?supported|different.?version|nao.?suporta|versao.?diferente/i.test(t);
    const hasMsPhone = /ms-phone:/g.test(t);
    const hasMsPhoneLink = /ms-phone-link:/g.test(t);
    const hasAka = /aka\\.ms/i.test(t);
    const hasQR = !!document.querySelector("canvas");
    const hasNdef = /base64|NTAG|ndef/i.test(t);
    const bleToggleBtns = [...document.querySelectorAll("button")].filter(b => /adver/i.test(b.textContent)).length;
    const pairingBtn = [...document.querySelectorAll("button")].filter(b => /phone link|pair/i.test(b.textContent)).length;
    return JSON.stringify({hasExcuse, hasMsPhone, hasMsPhoneLink, hasAka, hasQR, hasNdef, bleToggleBtns, pairingBtn, snapshot: t.slice(0, 2600)}, null, 1);
  })()`,

  // Click the "Open Phone Link pairing" NFC button (invokes nfc_open_link -> ms-phone: scheme)
  clickPairing: () => `(()=>{
    const b = [...document.querySelectorAll("button")].find(x => /Open Phone Link pairing/i.test(x.textContent));
    if (!b) return "ERR:button-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  // Check for toast/error about "app não existe" right after click
  pairResult: () => `(()=>{
    const t = document.body.innerText;
    const err = /n\u00e3o existe|does not exist|no app|not found.*app|nenhum app|n\u00e3o h\u00e1 app/i.test(t);
    const toast = document.querySelector("[class*=toast], [class*=alert], [class*=error]")?.textContent?.trim().slice(0, 200) || null;
    return JSON.stringify({ appMissingError: err, toast, tail: t.slice(-500) });
  })()`,

  // BLE advertise toggle card + scan button present?
  bleCard: () => `(()=>{
    const t = document.body.innerText;
    const adv = /BLE Advertising|Advertise PC over BLE|advertis/i.test(t);
    const scan = /Scan now/i.test(t);
    const btns = [...document.querySelectorAll("button")].filter(b => /adver/i.test(b.textContent)).map(b => b.textContent.trim());
    return JSON.stringify({ advCard: adv, scanBtn: scan, advButtons: btns });
  })()`,

  // Click the BLE "Scan now" button
  scanNow: () => `(()=>{
    const b = [...document.querySelectorAll("button")].find(x => /Scan now/i.test(x.textContent));
    if (!b) return "ERR:button-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  // Click Rescan in the modal
  rescan: () => `(()=>{
    const b = [...document.querySelectorAll("button")].find(x => /Rescan/i.test(x.textContent));
    if (!b) return "ERR:button-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  // Dump modal body text (between title and footer buttons)
  readModal: () => `(()=>{
    const t = document.body.innerText;
    const i = t.indexOf("Scan for Bluetooth devices");
    const j = t.indexOf("LocalSend");
    return JSON.stringify(t.slice(i, j > i ? j : i + 1500));
  })()`,

  // Direct backend test: call ble_discover via Tauri invoke and return result (async-friendly)
  invokeBle: () => `(async () => {
    try {
      const t = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
      if (!t) return "ERR:no-invoke-api";
      const res = await t("ble_discover", { seconds: 1 });
      return JSON.stringify(res, null, 1);
    } catch (e) {
      return "ERR:" + String(e);
    }
  })()`,

  // Dump the modal's innerHTML (last big overlay div with role=dialog or fixed position)
  modalHtml: () => `(()=>{
    const d = [...document.querySelectorAll("div")].find(x => x.textContent.includes("Scan for Bluetooth devices") && x.querySelector("button") && x.textContent.length < 6000 && x.textContent.includes("Cancel"));
    if (!d) return "ERR:modal-not-found";
    return d.innerHTML.slice(0, 5000);
  })()`,

  // Exact modal body text (smallest div wrapping title..Cancel)
  modalBody: () => `(()=>{
    const d = [...document.querySelectorAll("div")].filter(x => x.textContent.includes("Scan for Bluetooth devices") && x.textContent.includes("Cancel") && x.textContent.length < 5000).sort((a,b) => a.textContent.length - b.textContent.length);
    if (!d.length) return "ERR:modal-not-found";
    return JSON.stringify(d[0].textContent.slice(0, 3000));
  })()`,

  // Trigger invoke from UI thread but return immediately (fire-and-forget) and read back
  fireBle: () => `(()=>{
    window.__qaBleResult = "pending";
    (async () => {
      try {
        const t = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
        const res = await t("ble_discover", { seconds: 2 });
        window.__qaBleResult = JSON.stringify(res);
      } catch (e) {
        window.__qaBleResult = "ERR:" + String(e);
      }
    })();
    return "started";
  })()`,

  readBle: () => `(()=> JSON.stringify(window.__qaBleResult))()`,

  // ---- Performance tab (front 4: async mode apply, no requestAnimationFrame-jump) ----

  // Click the Performance tab in the sidebar/nav
  perfTab: () => `(()=>{
    const b = [...document.querySelectorAll("button, [role=tab], a")].find(x => /Performance/i.test(x.textContent) && x.textContent.length < 40);
    if (!b) return "ERR:perf-tab-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  // Dump current performance state: mode label, applying, dirty flag, buttons text
  perfDump: () => `(()=>{
    const t = document.body.innerText;
    const modeBtns = [...document.querySelectorAll("button")].filter(b => /(Balanced|Performance|Battery|Ultra|Power|Eco|Standard|Turbo)/i.test(b.textContent) && b.textContent.length < 30).map(b => ({txt: b.textContent.trim(), title: b.getAttribute("title") || "", aria: b.getAttribute("aria-label") || ""}));
    const applyingEl = [...document.querySelectorAll("[class*=applying], [class*=pulse]")].map(x => x.className);
    const dirty = window.__perfWriteDirtyUntil !== undefined ? window.__perfWriteDirtyUntil : null;
    return JSON.stringify({ tail: t.slice(-1400), modeBtns, applyingEl, dirty });
  })()`,

  // Click the "Balanced" (or given) mode button, then immediately snapshot state (no long sleep)
  perfClickBalanced: () => `(()=>{
    const b = [...document.querySelectorAll("button")].find(x => /Balanced/i.test(x.textContent) && x.textContent.length < 30);
    if (!b) return "ERR:balanced-btn-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  perfClickUltra: () => `(()=>{
    const b = [...document.querySelectorAll("button")].find(x => /Ultra|Turbo/i.test(x.textContent) && x.textContent.length < 30);
    if (!b) return "ERR:ultra-btn-not-found";
    b.click();
    return "clicked:" + b.textContent.trim();
  })()`,

  // Mouse hover over body to de-focus + check toast
  perfToast: () => `(()=>{
    const t = document.body.innerText;
    const toast = document.querySelector("[class*=toast], [class*=alert], [class*=snack]" )?.textContent?.trim().slice(0, 200) || null;
    return JSON.stringify({ tail: t.slice(-900), toast });
  })()`,
};

if (!scenarios[scenario]) {
  console.error('unknown scenario: ' + scenario);
  process.exit(1);
}
const jscode = scenarios[scenario]();
const payload = JSON.stringify({ window_label: 'main', code: jscode });
const r = spawnSync(process.execPath, [process.cwd() + '/qa_socket.mjs', 'execute_js', payload], {
  cwd: process.cwd(),
  encoding: 'utf8',
  shell: false,
  timeout: 30000,
});
console.log(r.stdout);
if (r.stderr) console.error(r.stderr);
