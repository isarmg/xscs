import { createXcssWebViteConfig } from "./xcss-web-vite.mjs";
import { isAbsolute, join } from "node:path";
import { pathToFileURL } from "node:url";
import { xcssFontLicenses } from "./font-licenses.mjs";
import { mergeConfig } from "vite";

async function runtimeInventoryPlugins() {
  const tools = process.env.XSSI_BUILD_TOOLS_DIR;
  const output = process.env.XSSI_FRONTEND_METADATA_FILE;
  if (!tools && !output) return [];
  if (!tools || !output || !isAbsolute(tools) || !isAbsolute(output)) {
    throw new Error("Explicit runtime inventory requires absolute tools and metadata paths");
  }
  const { runtimeInventory } = await import(pathToFileURL(join(tools, "scripts/frontend_build.mjs")).href);
  return [runtimeInventory({ output, root: process.cwd() })];
}

export default async function configuration() {
  const inventory = await runtimeInventoryPlugins();
  return mergeConfig(createXcssWebViteConfig(), {
  plugins: [...inventory, xcssFontLicenses()],
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:18104",
        // Keep Host aligned with the browser's Origin for login and CSRF checks.
        changeOrigin: false,
      },
    },
  },
});
}
