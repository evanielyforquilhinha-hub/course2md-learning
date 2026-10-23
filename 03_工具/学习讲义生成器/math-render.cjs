// Local, static formula rendering. No browser code or remote resources.
const fs = require('node:fs');
const katex = require('./vendor/katex/dist/katex.js');
const items = JSON.parse(fs.readFileSync(0, 'utf8'));
const result = items.map(item => {
  try {
    if (/\\(?:href|url|includegraphics|html\w*|def|gdef|newcommand)\b/.test(item.tex)) {
      throw new Error('公式包含不允许的链接、HTML或宏定义命令');
    }
    return {html: katex.renderToString(item.tex, {
      displayMode: item.display, output: 'htmlAndMathml', throwOnError: true,
      strict: 'error', trust: false, maxExpand: 200, maxSize: 20,
    })};
  } catch (error) { return {error: String(error.message)}; }
});
process.stdout.write(JSON.stringify(result));
