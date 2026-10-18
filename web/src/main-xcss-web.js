import "../xcss-web/vendor/xcss-web/startup.css";
import { startApplication } from "../xcss-web/vendor/xcss-web/startup.js";
// Bootstrap only. Application state, networking and views live in web/xcss-web.
import "@xcss/web/design-tokens/tokens.css";
import "@xcss/web/design-tokens/tokens.dark.css";
import "@xcss/web/design-tokens/reset.css";
import "@xcss/web/design-tokens/accessibility.css";
import "@xcss/web/web-fonts/fonts.css";
import "@xcss/web/admin-ui/styles.css";
import "@xcss/web/admin-ui/date-range.css";
import "./rustdesk.css";
import wasmUrl from "../pkg/xscs_web_bg.wasm?url";
await startApplication(async () => {
  const { default: init } = await import("../pkg/xscs_web.js");
  await init({ module_or_path: wasmUrl });
});
