const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', 'src', 'i18n');
const texts = {
  en: { debloatWorking: 'Working…' },
  pt: { debloatWorking: 'Aplicando…' },
  es: { debloatWorking: 'Aplicando…' },
  fr: { debloatWorking: 'Application…' },
};
for (const [locale, keys] of Object.entries(texts)) {
  const file = path.join(dir, `${locale}.json`);
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  Object.assign(data.sysOpt, keys);
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n');
  console.log(`${locale}: OK`);
}
