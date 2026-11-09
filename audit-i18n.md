# Auditoria de traduções (i18n) — MiControl

**Branch:** `f4c/i18n-audit` · **Base:** `integrate/audit-fixes` @ `690f9cc`
**Âmbito:** `src/i18n/{en,pt,es,fr}.json` + `src/hooks/useI18n.ts` + todo o `src/**/*.ts(x)` da aplicação.

---

## 1. Sumário

| Métrica | Antes | Depois |
|---|---|---|
| Chaves por locale | 1431 | **1673** (+278 novas, −36 órfãs) |
| Faltas de chave (check-i18n) | 0 | **0** |
| Strings vazias | 0 | **0** |
| Placeholders `{x}` desalinhados vs `en` | 0 | **0** |
| Texto user-facing hardcoded (app, fora de testes/landing) | **240** ocorrências | **59** (todas unidades/nomes técnicos) |
| `face.tsx` hardcoded | 92 | **0** |
| Chaves sem referência literal no código | 352 | 242 (dinâmicas ou pendentes de decisão) |
| Órfãs confirmadas removidas | — | **36** (× 4 locales; 38 chaves apagadas, 2 delas criadas e descartadas nesta auditoria) |

Os quatro locales já tinham **paridade de chaves** — o problema real não era o `check-i18n`, era
**(a)** um ecrã inteiro (Face Unlock) e dois painéis IoT escritos em inglês fixo, **(b)** chaves
mortas de funcionalidades removidas e **(c)** pequenas superfícies (toasts, `aria-label`, `title`,
placeholders) fora do sistema.

---

## 2. Metodologia

1. **Inventário** — `deepKeys` sobre os 4 JSON + comparação de conjuntos e valores.
2. **Uso real** — extração de todas as chamadas `t('…')` e de chaves passadas em arrays de configuração;
   exclusão de prefixos dinâmicos (`t(\`performance.descriptions.${m.descKey}\`)`, etc.).
3. **Hardcoded** — análise AST com o compilador TypeScript (não grep): nós `JsxText`, atributos
   `placeholder|title|aria-label|alt|label|desc`, `window.confirm/alert`, `new Error(...)` e
   propriedades `title|message|label|desc` em objetos de configuração.
4. **Qualidade** — vazios, paridade de `{placeholders}`, valores iguais ao inglês, heurística de
   frases inglesas em pt/es/fr (stopwords), tom/género.
5. **Correção + verificação** — `check-i18n`, `tsc --noEmit`, `eslint`, `vitest`, `vite build`.

---

## 3. Por locale

| Locale | Chaves antes | Chaves depois | Faltas antes | Faltas depois | Vazias | Placeholders | Iguais ao EN |
|---|---:|---:|---:|---:|---:|---:|---:|
| `en` (fonte) | 1431 | 1673 | — | — | 0 | 0 | — |
| `pt` | 1431 | 1673 | 0 | **0** | 0 | 0 | 73 |
| `es` | 1431 | 1673 | 0 | **0** | 0 | 0 | 74 |
| `fr` | 1431 | 1673 | 0 | **0** | 0 | 0 | 97 |

Os valores "iguais ao EN" são **termos que não se traduzem**: `MiControl`, `CPU`, `GPU`, `PID`,
`RPM`, `Wi-Fi`, `IoT WiFi`, `PSR2 DRRS`, `GitHub`, `mAh`, `Wh`, `Hz`, `TDP`, `AILM`, `LBLM`,
`Windows Hello`, siglas de protocolo. A varredura de frases inglesas em pt/es/fr devolve **0**.

Correções de valor feitas:

| Chave | Antes (fr/pt) | Depois |
|---|---|---|
| `performance.monitor.tdpCapLine` | `cap ~{value} W` em pt/es/fr | `limite ~{value} W` / `límite` / `plafond` |
| `common.minutes`, `_one`, `_other` | inglês em fr (correto) e pt/es com sync | traduzidos/verificados nos 4 |
| `common.items_one` (es/fr) | `{count} item` (inglês) | `{count} elemento` / `{count} élément` |
| `crossDevice.nfcCustomActionPlaceholder` | frase inglesa em fr | mantido com parênteses técnicos traduzidos |

---

## 4. Chaves órfãs

- **352** chaves nunca aparecem como literal `t('…')`.
- Após excluir prefixos dinâmicos (`sysOpt.items.*`, `performance.descriptions.*`,
  `performance.userHint.*`, `performance.techDetails.modes.*`, `battery.*`, `theme.*`,
  `fan.modes.*`, `touchpad.levels.*`) e variantes `_one`/`_other` cujo singular é usado, sobram **242**.
- Dessas, **38 chaves foram confirmadas mortas** (36 já existiam na base, 2 foram criadas e descartadas aqui) (grep sem qualquer referência, funcionalidade já removida do
  UI) e **removidas dos 4 locales**:

| Grupo | Chaves | Porquê |
|---|---|---|
| `display.drrsEnabled/drrsPendingReboot/drrsDisabled` | 3 | Substituídas por `display.psr2*` (eram duplicados exactos, nunca usados) |
| `about.bridge*` | 10 | O cartão "High Bridge — Silent Update" já não existe no UI |
| `updates.xpm*`, `updates.triggerScan`, `updates.nucleusNote` | 5 | Painel do XiaomiPCManager removido |
| `charging.protectMode`, `protectModeDesc`, `levels.40…80` | 7 | UI antiga de proteção de bateria; hoje usa `chargingThreshold` |
| `color.installedProfiles`, `loadSuccess`, `unloadSuccess`, `unloadError` | 4 | Fluxo ICC removido |
| `audio.loadingDevices`, `initError` | 2 | Substituídas por estados do componente |
| `cast.tip`, `cast.tipText` | 2 | Dica removida |
| `startup.autoStart`, `autoStartDesc` | 2 | Toggle removido |
| `face.single_rgb_camera`, `performance.smartVsAccel`, `iot.module.writeProfileHint` | 3 | Criadas/substituídas durante esta auditoria e não usadas |

As **242 restantes são mantidas de propósito**: ou são geradas dinamicamente a partir de ids do
backend (`sysOpt.items.<id>.{title,desc,why}` vem de `get_sys_opt_status`), ou são variantes de
enums (`security.status*`, `crossDevice.ls*`, `common.devices_*`). Apagá-las produziria chaves em
bruto no ecrã em máquinas com outro hardware/perfil.

---

## 5. Texto hardcoded encontrado e corrigido

Total na aplicação (excluindo `__tests__` e `landing/`): **240 → 59** ocorrências.

| Ficheiro | Antes | Depois | Exemplos (linha original) |
|---|---:|---:|---|
| `src/pages/tabs/face.tsx` | 92 | **0** | `276 Enroll face — ${stepTitle[step]}` · `333 Which Windows account…` · `796 window.confirm(…)` |
| `src/components/IotDeviceCard.tsx` | 41 | **0** | `114 IoT Device` · `128 IoT Service not available…` · `239 Cloud Binding — Indicates whether…` |
| `src/components/IotModulePanel.tsx` | 24 | 11 | `259 IoT Module` · `487 Write Controls (known-safe registers)` · `718 window.confirm(WARNING…)` |
| `src/components/ProcessModal.tsx` | 10 | **0** | `70 Top Processes — ${resource}` · `216 No processes found` · `225 Refreshes every 3 s…` |
| `src/pages/MainWindow.tsx` | 9 | 3 | `348 Theme: ${themeMode}` · `356 Preview tray popup` · `628 Preview — click outside to close` |
| `src/components/DisplaySettings.tsx` | 5 | 2 | `462 Dynamic Refresh Rate active — …` · `505 PSR2 DRRS ≠ Windows "Dynamic Refresh Rate"…` |
| `src/components/SystemInfoCard.tsx` | 5 | 3 | `29 Show top processes` · `78 GPU Usage` |
| `src/pages/tabs/about.tsx` | 5 | 2 | `66 MiControl Contributors` · `70 MIT License` · `74 GitHub Repository` |
| `src/pages/tabs/crossdevice.tsx` | 5 | 4 | `1220 aria-label="Close"` |
| `src/pages/TrayPopup.tsx` | 4 | 1 | `244 Audio` · `275 WiFi` · `278 → WiFi Manager` |
| `src/components/WiFiManager.tsx` | — | 0 | toasts `Connected to "…"`, `Connect error: …`, `Disconnected`, `Disconnect error: …` |
| `src/lib/brightnessPresets.ts` | — | 0 | `Eco/Standard/Vivid` + hints (usados em `DisplaySettings` e `TrayPopup`) |
| `src/pages/tabs/keyboard.tsx` | 2 | **0** | `248 Press the physical key to detect its VK code` · `259 Clear this key binding` |
| `src/contexts/ToastContext.tsx` | 2 | 1 | `99 aria-label="Dismiss"` |
| `src/components/OnboardingWizard.tsx` | 1 | **0** | `106 aria-label="Onboarding progress"` |
| `src/hooks/useAutoUpdate.ts` | 1 | **0** | `136 Downloaded installer not found after update download…` |
| `src/pages/tabs/ecrdebug.tsx` | 1 | **0** | `7 title="EC Debug Panel"` |
| `src/pages/tabs/performance.tsx` | 1 | **0** | `596 Path` |
| `src/components/AiConfigForm.tsx` | 3 | 2 | `216 e.g. llama3, mistral, gpt-4-vision-preview` (os outros são `sk-...` e um URL) |

**As 59 restantes são intencionais**: unidades (`°C`, `W`, `Hz`, `GB`, `dBm`, `s`), endereços hex
(`0xFE0B0300`, `DEADBEEF`), registos técnicos (`WMI`, `SMMT`, `SMMD`, `QFAN`, `control_flags[0x1B]`),
marcas (`MiControl`, `Xiaomi Laptop Pro`, `GitHub`), códigos internos (`consent_denied`, `no_logs`)
e falsos positivos onde já existe `t()`.

---

## 6. Chaves novas (resumo)

| Namespace | Chaves | Conteúdo |
|---|---:|---|
| `face.*` | 152 (160 no total) | wizard de registo (4 passos), estados/fase do herói, checklist, definições, manutenção, toasts, lista de modelos |
| `iot.*` | 78 | cartão de dispositivo, notas explicativas, tabela de 16 comandos EC, painel ECRAM (leitura/escrita crua) |
| `process.*` | 11 | modal de processos |
| `display.psr2*`, `display.drrActive` | 8 | nota DRR/PSR2 DRRS |
| `brightness.presets.*` | 6 | presets de brilho |
| `wifi.toast.*` | 4 | ligação/desligação WiFi |
| `main.*`, `common.dismiss`, `system.gpuUsage`, `keyboard.*Title`, `about.*`, `updates.path`, `onboarding.progress`, `aiConfig.modelPlaceholder` | 20 | restantes superfícies |

Qualidade: PT-PT (`câmara`, `ecrã`, `palavra-passe`, `ficheiro`, `guardar`, `iniciar sessão`,
`utilizadores`), ES e FR com concordância de género e tratamento formal consistente
(`usted` / `vous`). Pluralização usa a convenção existente `_one` / `_other`
(`face.phase.ready.title`, `face.checklist.faceCount`, `face.settings.renewEvery`,
`face.settings.lockoutNote`, `face.templates.count`), com a chave-base criada para o fallback.

---

## 7. Decisões

1. **Ênfase inline (`<b>`) dentro de frases** — `t()` devolve string simples e não há componente de
   rich-text. As 4 frases do wizard que misturavam `<b>` com texto passaram a uma chave única, sem
   negrito interno. Preferiu-se tradução correta e completa a partir frases em 5 fragmentos, que
   produzem gramática errada em ES/FR.
2. **`landing/` fora do âmbito** — é um bundle separado (`vite.config.landing.ts`), sem
   `useI18n` nem deteção de idioma: página de marketing deliberadamente monolingue (56 ocorrências).
3. **Nomes de produto não traduzidos** — `Smart vs Smart Acceleration`, `PSR2 DRRS`,
   `Intel Adaptive Refresh Rate`, `MiControl`, `Xiaomi Laptop Pro`.
4. **Unidades e valores técnicos** mantidos literais; `process.ram` usa `RAM MB` (pt/es) e `RAM Mo` (fr).
5. **`types/error.ts`** mantém `'An unexpected error occurred'` como fallback de último recurso para
   objectos que não são `Error`; é convertido em `errors.unexpected` a montante na maioria dos fluxos.
6. **Órfãs** só foram apagadas após grep sem qualquer referência **e** confirmação de que a
   funcionalidade saiu do UI; as dinâmicas ficaram.

---

## 8. Verificação

| Comando | Resultado |
|---|---|
| `node scripts/check-i18n.mjs` | ✅ `no missing keys, empty values or placeholder drift` |
| `npx tsc --noEmit` | ✅ 0 erros |
| `npm run lint` (`eslint src`) | ✅ 0 erros |
| `npx vitest run` | ✅ 35 ficheiros, **284 testes** |
| `npm run build` (`tsc && vite build`) | ✅ built in 2.06 s |

O `check-i18n.mjs` foi reforçado: além de chaves em falta, passa a falhar com **valores vazios** e
**placeholders desalinhados**. Foi acrescentado `src/__tests__/i18nParity.test.ts` (paridade de
chaves, vazios, placeholders e cobertura de Face Unlock/IoT) e o script `npm run i18n:check`.

---

## 9. Ficheiros alterados

- **i18n:** `src/i18n/en.json`, `pt.json`, `es.json`, `fr.json`
- **Código:** `pages/tabs/face.tsx`, `ecrdebug.tsx`, `keyboard.tsx`, `performance.tsx`, `about.tsx`,
  `crossdevice.tsx`; `pages/MainWindow.tsx`, `TrayPopup.tsx`; `components/IotDeviceCard.tsx`,
  `IotModulePanel.tsx`, `ProcessModal.tsx`, `SystemInfoCard.tsx`, `DisplaySettings.tsx`,
  `WiFiManager.tsx`, `AiConfigForm.tsx`, `OnboardingWizard.tsx`; `contexts/ToastContext.tsx`;
  `hooks/useAutoUpdate.ts`; `lib/brightnessPresets.ts`
- **Infra:** `scripts/check-i18n.mjs`, `package.json` (`i18n:check`)
- **Testes:** `src/__tests__/i18nParity.test.ts` (novo)