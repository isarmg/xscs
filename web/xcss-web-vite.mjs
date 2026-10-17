import { xcssWebStaticAssets } from "./xcss-web-assets.mjs";
import { readFileSync } from "node:fs";
// Existing asset/license/inventory plugins remain composed by vite.config.ts.
export function createXcssWebViteConfig(options = {}) {
  const maximum = options.maxAssetBytes ?? 512 * 1024;
  return {
    ...(options.base === undefined ? {} : { base: options.base }),
    plugins: [{ name: "xcss-web-entry", transformIndexHtml: { order: "pre", handler(html) { return html.replace('/src/main.tsx', '/src/main-xcss-web.js'); } } },
      { name: "xcss-web-build-provenance", generateBundle() { this.emitFile({ type: "asset", fileName: "assets/xcss-web-build.json", source: readFileSync(new URL("./pkg/xcss-web-build.json", import.meta.url)) }); } },
      xcssWebStaticAssets(),
      { name: "xcss-asset-budget", enforce: "post", generateBundle(_options, bundle) {
        let total = 0;
        for (const [name, asset] of Object.entries(bundle)) {
          const source = asset.type === "chunk" ? asset.code : asset.source;
          const size = typeof source === "string" ? new TextEncoder().encode(source).byteLength : source.byteLength;
          total += size;
          const limit = asset.type === "asset" && /\.woff2?$/i.test(name) ? 16 * 1024 * 1024 : /\.wasm(?:\.gz)?$/i.test(name) ? 4 * 1024 * 1024 : maximum;
          if (size > limit) this.error(`Asset ${name} exceeds the ${limit}-byte xcss budget (${size} bytes)`);
          if (name.endsWith(".map")) this.error("xcss web builds must not publish source maps");
        }
        if (total > 16 * 1024 * 1024) this.error(`Web assets exceed the 16-MiB budget (${total} bytes)`);
        if (!Object.keys(bundle).some(name => name.endsWith(".wasm"))) this.error("The xcss-web build must emit an independent WASM asset");
      } }],
    build: { target: "esnext", cssTarget: "esnext", modulePreload: { polyfill: false }, outDir: process.env.XCSS_WEB_DIST ?? "dist", emptyOutDir: true, sourcemap: false, chunkSizeWarningLimit: Math.floor(maximum / 1024) },
  };
}
