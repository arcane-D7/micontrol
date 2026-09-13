const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', 'src', 'i18n');
const texts = {
  en: { stateApplied: 'APPLIED', stateDefault: 'Windows default' },
  pt: { stateApplied: 'APLICADO', stateDefault: 'Padrão do Windows' },
  es: { stateApplied: 'APLICADO', stateDefault: 'Predeterminado de Windows' },
  fr: { stateApplied: 'APPLIQUÉ', stateDefault: 'Défaut Windows' },
};
for (const [locale, keys] of Object.entries(texts)) {
  const file = path.join(dir, `${locale}.json`);
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  Object.assign(data.sysOpt, keys);
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n');
  console.log(`${locale}: OK`);
}
