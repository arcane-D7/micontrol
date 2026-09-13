// S55 impeccable audit — i18n hardcode fixer for system/face/setup tabs.
// Adds locale keys and rewrites hardcoded JSX text to t() calls.
// One-shot (archived to qa/archive by design).
import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();

// ── Translations ─────────────────────────────────────────────────────────────
// en = source of truth; pt/es/fr translated; technical EC names (SMMT, QFAN,
// control_flags) stay in English in every locale — they're protocol names.
const STRINGS = {
  system: {
    'AC connected': { pt: 'AC conectado', es: 'AC conectado', fr: 'Secteur connecté' },
    'Adapter power': {
      pt: 'Potência do adaptador',
      es: 'Potencia del adaptador',
      fr: 'Puissance de l’adaptateur',
    },
    'Battery current': {
      pt: 'Corrente da bateria',
      es: 'Corriente de la batería',
      fr: 'Courant de la batterie',
    },
    'Battery voltage': {
      pt: 'Tensão da bateria',
      es: 'Voltaje de la batería',
      fr: 'Tension de la batterie',
    },
    'Battery capacity': {
      pt: 'Capacidade da bateria',
      es: 'Capacidad de la batería',
      fr: 'Capacité de la batterie',
    },
    'Charge limit': { pt: 'Limite de carga', es: 'Límite de carga', fr: 'Limite de charge' },
    'Battery temp': {
      pt: 'Temp. da bateria',
      es: 'Temp. de la batería',
      fr: 'Temp. de la batterie',
    },
    'CPU temp': { pt: 'Temp. da CPU', es: 'Temp. de la CPU', fr: 'Temp. du CPU' },
    'CPU power': { pt: 'Potência da CPU', es: 'Potencia de la CPU', fr: 'Puissance du CPU' },
    'Fan 1 RPM': { pt: 'RPM ventilador 1', es: 'RPM ventilador 1', fr: 'RPM ventilateur 1' },
    'Fan 2 RPM': { pt: 'RPM ventilador 2', es: 'RPM ventilador 2', fr: 'RPM ventilateur 2' },
    'Performance profile': {
      pt: 'Perfil de desempenho',
      es: 'Perfil de rendimiento',
      fr: 'Profil de performance',
    },
    'TDP override': { pt: 'Override de TDP', es: 'Anulación de TDP', fr: 'Dérogation TDP' },
    'Smart profile': {
      pt: 'Perfil inteligente',
      es: 'Perfil inteligente',
      fr: 'Profil intelligent',
    },
    'AI limit (AILM)': { pt: 'Limite IA (AILM)', es: 'Límite IA (AILM)', fr: 'Limite IA (AILM)' },
    'Long battery limit': {
      pt: 'Limite de bateria prolongada',
      es: 'Límite de batería prolongada',
      fr: 'Limite de batterie prolongée',
    },
    'Display brightness': {
      pt: 'Brilho da tela',
      es: 'Brillo de la pantalla',
      fr: 'Luminosité de l’écran',
    },
    'KB backlight': {
      pt: 'Retroiluminação do teclado',
      es: 'Retroiluminación del teclado',
      fr: 'Rétroéclairage du clavier',
    },
  },
  setup: {
    'AC connected': { pt: '', es: '', fr: '' },
    'Adapter power': { pt: '', es: '', fr: '' },
    'Battery current': { pt: '', es: '', fr: '' },
    'Battery voltage': { pt: '', es: '', fr: '' },
    'Battery capacity': { pt: '', es: '', fr: '' },
    'Charge limit': { pt: '', es: '', fr: '' },
    'Battery temp': { pt: '', es: '', fr: '' },
    'CPU temp': { pt: '', es: '', fr: '' },
    'CPU power': { pt: '', es: '', fr: '' },
    'Fan 1 RPM': { pt: '', es: '', fr: '' },
    'Fan 2 RPM': { pt: '', es: '', fr: '' },
    'Performance profile': { pt: '', es: '', fr: '' },
    'TDP override': { pt: '', es: '', fr: '' },
    'Smart profile': { pt: '', es: '', fr: '' },
    'AI limit (AILM)': { pt: '', es: '', fr: '' },
    'Long battery limit': { pt: '', es: '', fr: '' },
    'Display brightness': { pt: '', es: '', fr: '' },
    'KB backlight': { pt: '', es: '', fr: '' },
  },
  face: {
    'single RGB camera': {
      pt: 'câmera RGB única',
      es: 'cámara RGB única',
      fr: 'caméra RGB unique',
    },
    Diagnostics: { pt: 'Diagnóstico', es: 'Diagnóstico', fr: 'Diagnostic' },
    'Diagnostics error': {
      pt: 'Erro do diagnóstico',
      es: 'Error de diagnóstico',
      fr: 'Erreur de diagnostic',
    },
    'Unsaved changes': {
      pt: 'Alterações não salvas',
      es: 'Cambios sin guardar',
      fr: 'Modifications non enregistrées',
    },
    'Similarity threshold': {
      pt: 'Limiar de similaridade',
      es: 'Umbral de similitud',
      fr: 'Seuil de similarité',
    },
    'Show tile at sign-in': {
      pt: 'Mostrar bloco na tela de login',
      es: 'Mostrar mosaico en el inicio de sesión',
      fr: 'Afficher la tuile à l’ouverture de session',
    },
    'Re-enrollment reminder': {
      pt: 'Lembrete de recadastramento',
      es: 'Recordatorio de reregistro',
      fr: 'Rappel de réinscription',
    },
    'Anti-spoof threshold': {
      pt: 'Limiar antifalsificação',
      es: 'Umbral antifalsificación',
      fr: 'Seuil anti-usurpation',
    },
    'Face Unlock power': {
      pt: 'Ligar/desligar Face Unlock',
      es: 'Activar/desactivar Face Unlock',
      fr: 'Activer/désactiver Face Unlock',
    },
  },
};

function slug(s) {
  return s
    .toLowerCase()
    .replace(/\[|\]/g, '')
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_|_$/g, '');
}

function deepSet(obj, keyPath, value) {
  let cur = obj;
  for (let i = 0; i < keyPath.length - 1; i++) {
    if (typeof cur[keyPath[i]] !== 'object' || cur[keyPath[i]] === null) cur[keyPath[i]] = {};
    cur = cur[keyPath[i]];
  }
  cur[keyPath[keyPath.length - 1]] = value;
}

function processTab(tab) {
  const map = STRINGS[tab];
  if (!map) {
    console.log(`no string table for ${tab}, skipping`);
    return;
  }
  const tsxPath = path.join(root, 'src', 'pages', 'tabs', `${tab}.tsx`);
  let tsx = fs.readFileSync(tsxPath, 'utf8');
  let replaced = 0;

  // When the same strings already exist under another keyspace (system.*),
  // reuse those keys instead of duplicating translations.
  const keyPrefix = tab === 'setup' ? 'system' : tab;

  const en = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/en.json'), 'utf8'));
  const pt = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/pt.json'), 'utf8'));
  const es = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/es.json'), 'utf8'));
  const fr = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/fr.json'), 'utf8'));

  for (const [enStr, loc] of Object.entries(map)) {
    const key = slug(enStr);
    const needle = `>${enStr}<`;
    if (!tsx.includes(needle)) continue;
    if (tab !== 'setup') {
      deepSet(en, [tab, key], enStr);
      deepSet(pt, [tab, key], loc.pt);
      deepSet(es, [tab, key], loc.es);
      deepSet(fr, [tab, key], loc.fr);
    }
    tsx = tsx.split(needle).join(`>{t('${keyPrefix}.${key}')}<`);
    replaced++;
  }

  fs.writeFileSync(tsxPath, tsx);
  for (const [loc, obj] of [
    ['en', en],
    ['pt', pt],
    ['es', es],
    ['fr', fr],
  ]) {
    const p = path.join(root, 'src', 'i18n', `${loc}.json`);
    fs.writeFileSync(p, JSON.stringify(obj, null, 2) + '\n');
  }
  console.log(`${tab}: ${replaced} strings migrated`);
}

for (const tab of process.argv.slice(2)) processTab(tab);
