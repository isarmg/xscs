import { gzipSync } from "node:zlib";

// Compress once while building. Axum negotiates these verified static variants.
export function leptosStaticAssets() {
  let base = "/";
  return {
    name: "leptos-static-assets",
    enforce: "post",
    configResolved(config) { base = config.base; },
    generateBundle(_options, bundle) {
      for (const [fileName, asset] of Object.entries(bundle)) {
        if (asset.type !== "asset" || !fileName.endsWith(".wasm")) continue;
        const source = typeof asset.source === "string" ? Buffer.from(asset.source) : Buffer.from(asset.source);
        this.emitFile({type:"asset",fileName:`${fileName}.gz`,source:gzipSync(source,{level:9})});
      }
    },
    transformIndexHtml: {
      order: "post",
      handler(_html, context) {
        if (!context.bundle) return;
        return Object.keys(context.bundle).filter(name=>name.endsWith(".wasm")).map(name=>({
          tag:"link", attrs:{rel:"preload",href:`${base}${name}`,as:"fetch",type:"application/wasm",crossorigin:"anonymous"},injectTo:"head",
        }));
      },
    },
  };
}
