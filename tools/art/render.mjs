// Renders the README and social artwork in docs/art/*.html to PNGs in docs/assets/art/.
//
//   npm install --no-save playwright && npx playwright install chromium   (once)
//   node tools/art/render.mjs            # every piece
//   node tools/art/render.mjs hero       # just docs/art/hero.html
//
// Each page holds one fixed-size `.art` element. README pieces are rendered at 2x so they
// stay sharp on high-density screens; the social preview is rendered at exactly 1280x640,
// the size GitHub asks for.
import { chromium } from "playwright";
import { readdirSync, mkdirSync } from "node:fs";
import { dirname, join, basename } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const src = join(root, "docs", "art");
const out = join(root, "docs", "assets", "art");
mkdirSync(out, { recursive: true });

const only = process.argv.slice(2);
const pages = readdirSync(src)
  .filter((f) => f.endsWith(".html"))
  .filter((f) => only.length === 0 || only.includes(basename(f, ".html")));

const browser = await chromium.launch();
for (const file of pages) {
  const name = basename(file, ".html");
  const scale = name === "social-preview" ? 1 : 2;
  const page = await browser.newPage({ viewport: { width: 1600, height: 1200 }, deviceScaleFactor: scale });
  await page.goto(pathToFileURL(join(src, file)).href);
  await page.evaluate(() => document.fonts.ready);
  await page.locator(".art").screenshot({ path: join(out, `${name}.png`), omitBackground: true });
  await page.close();
  console.log(`docs/assets/art/${name}.png`);
}
await browser.close();
