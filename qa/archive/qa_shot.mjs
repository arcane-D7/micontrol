// Save a screenshot of the live app to qa_shot.png via MCP socket
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';

const r = spawnSync(
  process.execPath,
  [process.cwd() + '/qa_socket.mjs', 'take_screenshot', '{"window_label":"main"}'],
  { encoding: 'utf8', timeout: 40000 },
);
let j;
try {
  j = JSON.parse(r.stdout);
} catch {
  console.error('parse fail', r.stdout?.slice(0, 400));
  process.exit(1);
}
const b64 = j?.data?.data || j?.data?.image || '';
if (!b64) {
  console.error('no image; keys=', Object.keys(j?.data || {}));
  console.error(r.stdout?.slice(0, 600));
  process.exit(1);
}
const m = b64.match(/^data:image\/(\w+);base64,(.*)$/s);
const ext = m ? m[1] : 'png';
const raw = m ? m[2] : b64;
fs.writeFileSync('qa_shot.' + ext, Buffer.from(raw, 'base64'));
console.log('saved qa_shot.' + ext + ' bytes=' + Buffer.from(raw, 'base64').length);
