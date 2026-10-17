import { createServerCornerGeometry, offsetServerCornerPath } from "./server-corner-geometry.js";
export { createServerCornerGeometry, serverCornerProfile, offsetServerCornerPath, SERVER_CORNER_PROFILE, SERVER_CORNER_CONTROL_POINTS, SERVER_CORNER_PROFILE_BOUNDS, SERVER_CORNER_PHOTO_EXTENT } from "./server-corner-geometry.js";
const controllers = new WeakMap();
const xml = (s) => s.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" })[c]);
const number = (s) => Number.parseFloat(s) || 0;
const fmt = (n) => Number(n.toFixed(6)).toString();
const transitionNames = ["transition-property", "transition-duration", "transition-delay", "transition-timing-function", "transition-behavior"];
const ownedPaintProperty = (name) => /^(?:background(?:-|$)|border(?:-|$)|box-shadow$|mask(?:-|$)|backdrop-filter$)/.test(name);
const paintCache = new WeakMap();
function snapshot(css) {
    const properties = ["width", "height", "box-sizing", "display", "position", "overflow-x", "overflow-y", "border-image-source", "box-shadow", "background-color", "background-image", "backdrop-filter", "mask-image", "outline-style", "outline-width", "outline-offset", "--xcss-server-corner-extent", "--xcss-server-corner-mode", "--xcss-server-corner-foreground"];
    for (const side of ["top", "right", "bottom", "left"])
        properties.push(`padding-${side}`, `border-${side}-width`);
    const values = Object.fromEntries(properties.map(name => [name, css.getPropertyValue(name)]));
    for (const side of ["top", "right", "bottom", "left"]) {
        const visible = number(values[`border-${side}-width`]) > 0;
        values[`border-${side}-color`] = visible ? css.getPropertyValue(`border-${side}-color`) : "transparent";
        values[`border-${side}-style`] = visible ? css.getPropertyValue(`border-${side}-style`) : "none";
    }
    if (values["background-image"] !== "none" || values["backdrop-filter"] !== "none") {
        for (const name of ["background-position", "background-size", "background-repeat", "background-attachment", "background-origin", "background-clip", "background-blend-mode"])
            values[name] = css.getPropertyValue(name);
    }
    return new Proxy(values, { get(target, name) { return name === "getPropertyValue" ? (property) => target[property] ?? "" : target[String(name).replace(/[A-Z]/g, c => `-${c.toLowerCase()}`)]; } });
}
function split(value, separator) {
    const parts = [];
    let depth = 0, start = 0;
    for (let i = 0; i <= value.length; i++) {
        const c = value[i];
        if (c === "(")
            depth++;
        else if (c === ")")
            depth--;
        if (i === value.length || depth === 0 && (separator === "comma" ? c === "," : /\s/.test(c))) {
            const part = value.slice(start, i).trim();
            if (part)
                parts.push(part);
            start = i + 1;
        }
    }
    return parts;
}
function shadows(value) {
    if (value === "none")
        return [];
    const result = [];
    for (const layer of split(value, "comma")) {
        const tokens = split(layer, "space");
        if (tokens.includes("inset"))
            return null;
        const lengths = tokens.filter(t => /^-?(?:\d+\.?\d*|\.\d+)px$/.test(t)).map(Number.parseFloat);
        const colors = tokens.filter(t => !/^-?(?:\d+\.?\d*|\.\d+)px$/.test(t));
        if (lengths.length < 2 || lengths.length > 4 || colors.length !== 1)
            return null;
        result.push({ color: colors[0], x: lengths[0], y: lengths[1], blur: lengths[2] ?? 0, spread: lengths[3] ?? 0 });
    }
    return result;
}
function mask(geometry, margin) {
    const w = geometry.width, h = geometry.height, sw = w + margin * 2, sh = h + margin * 2;
    // White outside the border box keeps native focus and decoration outset.
    // The fixed host mask clips scaled/replaced children in host coordinates.
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${fmt(sw)}" height="${fmt(sh)}"><path d="M0 0H${fmt(sw)}V${fmt(sh)}H0ZM${margin} ${margin}H${fmt(w + margin)}V${fmt(h + margin)}H${margin}Z" fill="white" fill-rule="evenodd"/><path transform="translate(${margin} ${margin})" d="${geometry.path}" fill="white"/></svg>`;
}
function decoration(geometry, css, layers, hasBackgroundImage, foreground = false) {
    const { width: w, height: h, extent: e } = geometry;
    const sides = ["top", "right", "bottom", "left"];
    const widths = sides.map(side => number(css.getPropertyValue(`border-${side}-width`)));
    const colors = sides.map(side => css.getPropertyValue(`border-${side}-color`));
    const styles = sides.map(side => css.getPropertyValue(`border-${side}-style`));
    const key = JSON.stringify([widths, colors, styles, css.boxShadow, css.backgroundColor, hasBackgroundImage, foreground]);
    let cache = paintCache.get(geometry);
    if (!cache) {
        cache = new Map();
        paintCache.set(geometry, cache);
    }
    if (cache.has(key))
        return cache.get(key);
    if (styles.some((style, i) => widths[i] > 0 && !["solid", "dashed", "dotted", "none"].includes(style)))
        return null;
    if (geometry.canonical && !hasBackgroundImage && layers.length === 0 && widths.every(width => width === 0)) {
        // A nine-slice image stretches only the 1px straight centre strips. The
        // four Mac mini corners retain their physical dimensions, so equal-role tiles
        // with varying widths share one SVG and Blob URL without curve distortion.
        const compact = createServerCornerGeometry({ width: e * 2 + 1, height: e * 2 + 1, extent: e, tolerance: .02 });
        const margin = 2, size = e * 2 + 1 + margin * 2;
        const result = { image: `<svg xmlns="http://www.w3.org/2000/svg" width="${fmt(size)}" height="${fmt(size)}"><path transform="translate(${margin} ${margin})" d="${compact.path}" fill="${xml(css.backgroundColor)}"/></svg>`, margin, slice: e + margin };
        cache.set(key, result);
        if (cache.size > 16)
            cache.delete(cache.keys().next().value);
        return result;
    }
    const margin = Math.max(1, Math.ceil(Math.max(0, ...layers.map(s => Math.max(Math.abs(s.x), Math.abs(s.y)) + s.blur * 3 + Math.max(0, s.spread))) + 2));
    const svgWidth = w + margin * 2, svgHeight = h + margin * 2;
    let defs = `<clipPath id="inside"><path d="${geometry.path}"/></clipPath><mask id="outside" maskUnits="userSpaceOnUse" x="-${margin}" y="-${margin}" width="${svgWidth}" height="${svgHeight}"><rect x="-${margin}" y="-${margin}" width="${svgWidth}" height="${svgHeight}" fill="white"/><path d="${geometry.path}" fill="black"/></mask>`;
    let body = "";
    for (let i = layers.length - 1; i >= 0; i--) {
        const s = layers[i];
        const shadowPath = s.spread === 0 ? geometry.path : offsetServerCornerPath(geometry, s.spread);
        defs += `<filter id="shadow${i}" filterUnits="userSpaceOnUse" x="-${margin}" y="-${margin}" width="${svgWidth}" height="${svgHeight}" color-interpolation-filters="sRGB"><feGaussianBlur in="SourceAlpha" stdDeviation="${s.blur / 2}"/><feOffset dx="${s.x}" dy="${s.y}" result="blurred"/><feFlood flood-color="${xml(s.color)}"/><feComposite operator="in" in2="blurred"/></filter>`;
        body += `<path d="${shadowPath}" fill="black" filter="url(#shadow${i})" mask="url(#outside)"/>`;
    }
    if (!hasBackgroundImage)
        body += `<path d="${geometry.path}" fill="${xml(css.backgroundColor)}"/>`;
    const junction = (a, b, e) => [e * a / Math.max(a, b, 1e-9), e * b / Math.max(a, b, 1e-9)];
    const [tlx, tly] = junction(widths[3], widths[0], geometry.corners[3].extent);
    const [trx, try_] = junction(widths[1], widths[0], geometry.corners[0].extent);
    const [brx, bry] = junction(widths[1], widths[2], geometry.corners[1].extent);
    const [blx, bly] = junction(widths[3], widths[2], geometry.corners[2].extent);
    const polygons = [`0,0 ${w},0 ${w - trx},${try_} ${tlx},${tly}`,
        `${w},0 ${w},${h} ${w - brx},${h - bry} ${w - trx},${try_}`,
        `0,${h} ${w},${h} ${w - brx},${h - bry} ${blx},${h - bly}`,
        `0,0 ${tlx},${tly} ${blx},${h - bly} 0,${h}`];
    for (let i = 0; i < sides.length; i++) {
        if (foreground || widths[i] === 0 || styles[i] === "none")
            continue;
        defs += `<clipPath id="side${i}"><polygon points="${polygons[i]}"/></clipPath>`;
        const dash = styles[i] === "dashed" ? ` stroke-dasharray="${widths[i] * 3} ${widths[i] * 2}"` : styles[i] === "dotted" ? ` stroke-dasharray="0 ${widths[i] * 2}" stroke-linecap="round"` : "";
        body += `<g clip-path="url(#inside)"><path d="${geometry.path}" fill="none" stroke="${xml(colors[i])}" stroke-width="${widths[i] * 2}" clip-path="url(#side${i})"${dash}/></g>`;
    }
    const result = { image: `<svg xmlns="http://www.w3.org/2000/svg" width="${fmt(svgWidth)}" height="${fmt(svgHeight)}"><defs>${defs}</defs><g transform="translate(${margin} ${margin})">${body}</g></svg>`, margin };
    cache.set(key, result);
    if (cache.size > 16)
        cache.delete(cache.keys().next().value);
    return result;
}
/** Explicitly start server-only Mac mini decoration. No global import side effects.
 * Layout, native controls, hit boxes and focus outlines remain browser-owned.
 * Images/video and overflow-hidden containers use a fixed host mask. Its white
 * exterior keeps focus outlines/shadows outside the border box visible. */
export function startServerCorners(document) {
    const previous = controllers.get(document);
    if (previous)
        return previous;
    const ownerView = document.defaultView;
    if (!ownerView)
        throw new TypeError("Server corners require a live browser document");
    const view = ownerView;
    for (const registration of [
        { name: "--xcss-server-corner-extent", syntax: "<length-percentage>", inherits: false, initialValue: "0px" },
        { name: "--xcss-server-corner-mode", syntax: "*", inherits: false, initialValue: "paint" },
        { name: "--xcss-server-corner-foreground", syntax: "<number>", inherits: false, initialValue: "0" },
    ]) {
        try {
            view.CSS.registerProperty(registration);
        }
        catch (error) {
            if (!(error instanceof view.DOMException) || error.name !== "InvalidModificationError")
                throw error;
        }
    }
    const records = new Map();
    const draftStyle = document.createElement("div").style;
    const allProperties = Array.from(view.getComputedStyle(document.documentElement)).filter(name => !ownedPaintProperty(name));
    // Raw SVG may be cached across documents; Blob URLs belong to this controller
    // only. Never share a URL that another document's stop() could revoke.
    const imageUrls = new Map();
    let usedImages = new Set();
    const imageUrl = (source) => {
        usedImages.add(source);
        let url = imageUrls.get(source);
        if (!url) {
            url = view.URL.createObjectURL(new view.Blob([source], { type: "image/svg+xml" }));
            imageUrls.set(source, url);
        }
        return `url("${url}")`;
    };
    let frame = 0, stopped = false;
    // Count per target because its element and pseudo-elements can transition
    // concurrently. Detached targets cannot deliver cancellation to document.
    const activeExtentTransitions = new Map();
    const state = (element) => {
        let value = records.get(element);
        if (!value) {
            value = { saved: new Map(), pending: new Map(), transitionDirty: true, originalState: element.getAttribute("data-xcss-server-corner-state"), originalReason: element.getAttribute("data-xcss-server-corner-reason") };
            records.set(element, value);
        }
        return value;
    };
    const put = (element, name, value) => {
        const record = state(element);
        let saved = record.saved.get(name);
        if (!saved) {
            saved = { value: element.style.getPropertyValue(name), priority: element.style.getPropertyPriority(name) };
            record.saved.set(name, saved);
        }
        record.pending.set(name, value);
    };
    const restore = (element, record, final) => {
        // Exact inline text proves no author edit occurred. Reuse its previously
        // restored serialization while still reading every computed style below.
        if (!final && record.restoredStyle !== undefined && element.style.cssText === record.appliedStyle) {
            element.style.cssText = record.restoredStyle;
            for (const [name, saved] of record.saved)
                if (!transitionNames.includes(name))
                    saved.applied = undefined;
        }
        else {
            draftStyle.cssText = element.style.cssText;
            for (const [name, saved] of record.saved) {
                if (!final && transitionNames.includes(name))
                    continue;
                const value = element.style.getPropertyValue(name), priority = element.style.getPropertyPriority(name);
                // Once released, the current inline declaration belongs to the author.
                if (saved.applied === undefined || value !== saved.applied || priority !== "important") {
                    saved.value = value;
                    saved.priority = priority;
                }
                if (saved.value)
                    draftStyle.setProperty(name, saved.value, saved.priority);
                else
                    draftStyle.removeProperty(name);
                saved.applied = undefined;
            }
            if (element.style.cssText !== draftStyle.cssText)
                element.style.cssText = draftStyle.cssText;
        }
        record.pending.clear();
        if (record.decoration)
            record.decoration.style.display = "none";
        if (record.foreground)
            record.foreground.style.setProperty("display", "none", "important");
        if (final) {
            record.decoration?.remove();
            record.foreground?.remove();
            for (const [name, value] of [["data-xcss-server-corner-state", record.originalState], ["data-xcss-server-corner-reason", record.originalReason]]) {
                if (value === null)
                    element.removeAttribute(name);
                else
                    element.setAttribute(name, value);
            }
            records.delete(element);
        }
    };
    const prepareTransitions = (element, measured) => {
        const previous = records.get(element);
        if (previous && !previous.transitionDirty)
            return;
        // Read author transition configuration with paint overrides still in place;
        // no background change occurs while this configuration is restored.
        if (previous) {
            draftStyle.cssText = element.style.cssText;
            for (const name of transitionNames) {
                const saved = previous.saved.get(name);
                if (!saved)
                    continue;
                const current = element.style.getPropertyValue(name), priority = element.style.getPropertyPriority(name);
                if (saved.applied === undefined || current !== saved.applied || priority !== "important") {
                    saved.value = current;
                    saved.priority = priority;
                }
                if (saved.value)
                    draftStyle.setProperty(name, saved.value, saved.priority);
                else
                    draftStyle.removeProperty(name);
                saved.applied = undefined;
            }
            if (element.style.cssText !== draftStyle.cssText)
                element.style.cssText = draftStyle.cssText;
        }
        const css = measured ?? view.getComputedStyle(element);
        const properties = split(css.transitionProperty, "comma");
        const lists = transitionNames.slice(1).map(name => split(css.getPropertyValue(name), "comma"));
        // The UA default is `all 0s`, which needs no intervention. Expanding it on
        // every tile would add hundreds of inert declarations and force layouts.
        if (!properties.some((name, index) => (name === "all" || ownedPaintProperty(name)) && number(lists[0][index % lists[0].length]) > 0)) {
            if (previous)
                previous.transitionDirty = false;
            return;
        }
        const retained = [];
        properties.forEach((name, index) => {
            if (name === "all")
                retained.push(...allProperties.map(name => ({ name, index })));
            else if (!ownedPaintProperty(name) && name !== "none")
                retained.push({ name, index });
        });
        // SVG paints change palette at the state boundary. Keeping transitions on
        // the native transparent paint would produce a run/cancel loop on every
        // measurement. Geometry, opacity and transform animations remain native.
        put(element, "transition-property", retained.map(p => p.name).join(",") || "none");
        lists.forEach((list, index) => put(element, transitionNames[index + 1], retained.map(p => list[p.index % list.length] || (index === 3 ? "normal" : "0s")).join(",") || (index === 2 ? "ease" : index === 3 ? "normal" : "0s")));
        const record = state(element);
        record.transitionDirty = false;
        draftStyle.cssText = element.style.cssText;
        for (const [name, value] of record.pending)
            draftStyle.setProperty(name, value, "important");
        element.style.cssText = draftStyle.cssText;
        for (const name of record.pending.keys())
            record.saved.get(name).applied = element.style.getPropertyValue(name);
        record.pending.clear();
    };
    const mark = (element, status, reason) => {
        state(element);
        element.setAttribute("data-xcss-server-corner-state", status);
        if (reason)
            element.setAttribute("data-xcss-server-corner-reason", reason);
        else
            element.removeAttribute("data-xcss-server-corner-reason");
    };
    const positionForeground = (element, foreground) => {
        // A fixed child remains in its popover/modal stacking context but does not
        // scroll with the panel's option content. Its SVG screen matrix also lets
        // us position it in a transformed ancestor's containing-block coordinates.
        const matrix = foreground.getScreenCTM();
        if (!matrix)
            return;
        const determinant = matrix.a * matrix.d - matrix.b * matrix.c;
        if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-12)
            return;
        const box = element.getBoundingClientRect(), { width, height } = foreground.viewBox.baseVal;
        const x = box.left - Math.min(0, matrix.a * width) - Math.min(0, matrix.c * height) - matrix.e;
        const y = box.top - Math.min(0, matrix.b * width) - Math.min(0, matrix.d * height) - matrix.f;
        foreground.style.setProperty("left", `${fmt(number(foreground.style.left) + (matrix.d * x - matrix.c * y) / determinant)}px`, "important");
        foreground.style.setProperty("top", `${fmt(number(foreground.style.top) + (matrix.a * y - matrix.b * x) / determinant)}px`, "important");
    };
    const forced = view.matchMedia("(forced-colors: active)");
    const colorScheme = view.matchMedia("(prefers-color-scheme: dark)");
    const observer = new view.MutationObserver(() => { for (const record of records.values())
        record.transitionDirty = true; schedule(); });
    const resize = new view.ResizeObserver(() => schedule());
    const observe = () => { observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true }); };
    function update() {
        frame = 0;
        if (stopped)
            return;
        // All self writes happen synchronously while disconnected. App mutations
        // cannot interleave in this JS task; no self-induced observer loop remains.
        observer.disconnect();
        usedImages = new Set();
        for (const element of records.keys())
            if (element.isConnected)
                prepareTransitions(element);
        for (const [element, record] of records) {
            if (!element.isConnected)
                resize.unobserve(element);
            restore(element, record, !element.isConnected);
            if (record.decoration)
                record.decoration.style.display = "none";
        }
        const selected = new Set();
        const candidates = [];
        // Read every candidate before paint writes, avoiding a style/layout flush
        // per tile in large galleries. Unselected descendants inherit extent=0.
        for (const node of document.querySelectorAll("*")) {
            if (node.namespaceURI !== "http://www.w3.org/1999/xhtml" || node.hasAttribute("data-xcss-server-corner-decoration"))
                continue;
            const live = view.getComputedStyle(node);
            if (number(node.getAttribute("data-xcss-server-corner-extent") ?? live.getPropertyValue("--xcss-server-corner-extent")) <= 0)
                continue;
            if (!records.has(node))
                prepareTransitions(node, live);
            const css = snapshot(live);
            let rect;
            const measuredDimension = (axis) => {
                const fallback = () => (rect ??= node.getBoundingClientRect())[axis];
                if (!/^(?:\d+\.?\d*|\.\d+)px$/.test(css[axis]))
                    return fallback();
                const value = number(css[axis]);
                const sides = axis === "width" ? ["left", "right"] : ["top", "bottom"];
                const size = css.boxSizing === "border-box" ? value : value + sides.reduce((sum, side) => sum + number(css.getPropertyValue(`padding-${side}`)) + number(css.getPropertyValue(`border-${side}-width`)), 0);
                return size || fallback();
            };
            // Definite CSS dimensions already describe the untransformed border box.
            // Avoid a bounding-box read per tile in ordinary block galleries. Keep
            // the fragment check for both inline content and multi-column blocks.
            const fragments = node.getClientRects().length;
            candidates.push({ node, css, width: measuredDimension("width"), height: measuredDimension("height"), fragmented: fragments > 1, visible: fragments > 0 });
        }
        for (const { node, css, width, height, fragmented, visible } of candidates) {
            const raw = node.getAttribute("data-xcss-server-corner-extent") ?? css.getPropertyValue("--xcss-server-corner-extent").trim();
            const mode = node.getAttribute("data-xcss-server-corner") ?? css.getPropertyValue("--xcss-server-corner-mode").trim();
            if (mode === "none" || number(raw) <= 0)
                continue;
            selected.add(node);
            resize.observe(node);
            // A closed dropdown can retain definite CSS dimensions. It must lose
            // readiness even though those values remain positive while display:none.
            if (number(css.getPropertyValue("--xcss-server-corner-foreground")) > 0 && !visible) {
                mark(node, "fallback", "not-visible");
                continue;
            }
            if (forced.matches || width <= 0 || height <= 0 || ["table", "table-row", "table-cell"].includes(css.display) || fragmented) {
                mark(node, "fallback", forced.matches ? "forced-colors" : width <= 0 || height <= 0 ? "not-visible" : fragmented ? "fragmented-inline" : "table-layout");
                continue;
            }
            if (css.borderImageSource !== "none" || css.maskImage !== "none") {
                mark(node, "fallback", css.maskImage !== "none" ? "existing-mask" : "existing-border-image");
                continue;
            }
            const extent = raw.endsWith("%") ? number(raw) * Math.min(width, height) / 100 : number(raw);
            let geometry;
            try {
                geometry = createServerCornerGeometry({ width, height, extent, tolerance: 0.02 });
            }
            catch {
                mark(node, "fallback", "invalid-geometry");
                continue;
            }
            const layers = shadows(css.boxShadow);
            if (!layers) {
                mark(node, "fallback", "inset-or-unsupported-shadow");
                continue;
            }
            const hasImage = css.backgroundImage !== "none";
            const hasBackdrop = css.backdropFilter !== "none";
            const backgroundChild = hasImage || hasBackdrop;
            const media = ["IMG", "VIDEO", "CANVAS"].includes(node.tagName);
            // Native/replaced controls cannot receive a background child safely.
            if (backgroundChild && ["INPUT", "TEXTAREA", "SELECT", "IMG", "VIDEO", "CANVAS"].includes(node.tagName) && !media) {
                mark(node, "fallback", "native-background-image-or-backdrop");
                continue;
            }
            const clipping = mode === "clip" || css.overflowX === "hidden" || css.overflowX === "clip" || css.overflowY === "hidden" || css.overflowY === "clip";
            const borderWidth = number(css.getPropertyValue("border-top-width"));
            // Opt-in dropdown frames have one solid border. Keep all existing
            // unequal/dashed/native surface behavior outside this narrow contract.
            const foreground = number(css.getPropertyValue("--xcss-server-corner-foreground")) > 0 && clipping && borderWidth > 0 &&
                !["INPUT", "TEXTAREA", "SELECT", "IMG", "VIDEO", "CANVAS"].includes(node.tagName) &&
                ["top", "right", "bottom", "left"].every(side => css.getPropertyValue(`border-${side}-style`) === "solid" &&
                    number(css.getPropertyValue(`border-${side}-width`)) === borderWidth && css.getPropertyValue(`border-${side}-color`) === css.getPropertyValue("border-top-color"));
            let foregroundPath;
            if (foreground) {
                try {
                    foregroundPath = geometry.path + offsetServerCornerPath(geometry, -borderWidth);
                }
                catch {
                    mark(node, "fallback", "invalid-foreground-inset");
                    continue;
                }
            }
            let painted;
            try {
                painted = decoration(geometry, css, layers, backgroundChild && !media, foreground);
            }
            catch {
                mark(node, "fallback", "invalid-paint-geometry");
                continue;
            }
            if (!painted) {
                mark(node, "fallback", "unsupported-border-or-shadow-inset");
                continue;
            }
            const record = state(node);
            const paintedUrl = imageUrl(painted.image);
            const borderWidths = ["top", "right", "bottom", "left"].map(side => css.getPropertyValue(`border-${side}-width`));
            const visibleOutline = number(css.outlineWidth) > 0 && !["none", "hidden"].includes(css.outlineStyle);
            // Fractional native controls can leave a second right edge when the
            // full SVG is resampled through border-image and its outset is admitted
            // by the mask's white exterior. Paint that same SVG at its exact size
            // inside the border box instead. Real outside shadows/outlines retain
            // the existing outset renderer and no-clip mask.
            const exactControl = ["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(node.tagName) &&
                !backgroundChild && !foreground && layers.length === 0 && !visibleOutline && borderWidths.some(value => number(value) > 0);
            if (foregroundPath) {
                let child = record.foreground;
                if (!child) {
                    child = document.createElementNS("http://www.w3.org/2000/svg", "svg");
                    child.setAttribute("data-xcss-server-corner-decoration", "foreground");
                    child.setAttribute("aria-hidden", "true");
                    child.setAttribute("focusable", "false");
                    child.append(document.createElementNS("http://www.w3.org/2000/svg", "path"));
                    record.foreground = child;
                }
                if (child.parentNode !== node || node.lastElementChild !== child)
                    node.append(child);
                child.setAttribute("viewBox", `0 0 ${fmt(width)} ${fmt(height)}`);
                child.style.cssText = `all:initial!important;display:block!important;position:fixed!important;left:0!important;top:0!important;width:${fmt(width)}px!important;height:${fmt(height)}px!important;max-width:none!important;max-height:none!important;pointer-events:none!important;visibility:inherit!important;z-index:2147483647!important;overflow:visible!important;`;
                const path = child.firstElementChild;
                path.setAttribute("d", foregroundPath);
                path.style.cssText = `fill:${css.getPropertyValue("border-top-color")}!important;fill-rule:evenodd!important;stroke:none!important;`;
            }
            if (backgroundChild && !media) {
                let child = record.decoration;
                if (!child) {
                    child = document.createElement("div");
                    child.setAttribute("data-xcss-server-corner-decoration", "");
                    child.setAttribute("aria-hidden", "true");
                    record.decoration = child;
                }
                if (child.parentNode !== node)
                    node.prepend(child);
                child.style.cssText = "position:absolute;pointer-events:none;z-index:-1;margin:0;padding:0;border:0;display:block;";
                child.style.inset = ["top", "right", "bottom", "left"].map(side => `${-number(css.getPropertyValue(`border-${side}-width`))}px`).join(" ");
                child.style.boxSizing = "border-box";
                // Preserve the author's padding/content background positioning areas.
                // This absolute decoration still occupies only the original border box.
                for (const side of ["top", "right", "bottom", "left"]) {
                    child.style.setProperty(`border-${side}`, `${css.getPropertyValue(`border-${side}-width`)} solid transparent`);
                    child.style.setProperty(`padding-${side}`, css.getPropertyValue(`padding-${side}`));
                }
                for (const name of ["background-color", "background-image", "background-position", "background-size", "background-repeat", "background-attachment", "background-origin", "background-clip", "background-blend-mode"])
                    child.style.setProperty(name, css.getPropertyValue(name));
                // Negative stacking backgrounds otherwise cover the host's border.
                // Repeat the transparent SVG border layer above the original CSS
                // background layers; external images remain ordinary CSS, never SVG.
                child.style.backgroundImage = `${paintedUrl}, ${css.backgroundImage}`;
                child.style.backgroundPosition = `${-painted.margin}px ${-painted.margin}px, ${css.backgroundPosition}`;
                child.style.backgroundSize = `${fmt(width + painted.margin * 2)}px ${fmt(height + painted.margin * 2)}px, ${css.backgroundSize}`;
                child.style.backgroundRepeat = `no-repeat, ${css.backgroundRepeat}`;
                child.style.backgroundOrigin = `border-box, ${css.backgroundOrigin}`;
                child.style.backgroundClip = `border-box, ${css.backgroundClip}`;
                child.style.backgroundBlendMode = `normal, ${css.backgroundBlendMode}`;
                child.style.backdropFilter = css.backdropFilter;
                child.style.clipPath = `path("${geometry.path}")`;
                if (css.position === "static")
                    put(node, "position", "relative");
                put(node, "isolation", "isolate");
            }
            put(node, "background-color", "transparent");
            if (!media)
                put(node, "background-image", "none");
            if (hasBackdrop)
                put(node, "backdrop-filter", "none");
            put(node, "box-shadow", "none");
            put(node, "border-radius", "0px");
            for (const [i, side] of ["top", "right", "bottom", "left"].entries()) {
                put(node, `border-${side}-width`, borderWidths[i]);
                put(node, `border-${side}-style`, "solid");
                put(node, `border-${side}-color`, "transparent");
            }
            if (exactControl) {
                put(node, "background-image", paintedUrl);
                put(node, "background-size", `${fmt(width + painted.margin * 2)}px ${fmt(height + painted.margin * 2)}px`);
                put(node, "background-position", `${-painted.margin}px ${-painted.margin}px`);
                put(node, "background-repeat", "no-repeat");
                put(node, "background-attachment", "scroll");
                put(node, "background-origin", "border-box");
                put(node, "background-clip", "border-box");
                put(node, "background-blend-mode", "normal");
            }
            put(node, "border-image-source", exactControl ? "none" : paintedUrl);
            put(node, "border-image-slice", `${painted.slice ?? painted.margin} fill`);
            put(node, "border-image-width", `${painted.slice ?? painted.margin}px`);
            put(node, "border-image-outset", exactControl ? "0" : `${painted.margin}px`);
            put(node, "border-image-repeat", "stretch");
            if (clipping) {
                const margin = Math.max(painted.margin, Math.ceil(number(css.outlineWidth) + Math.abs(number(css.outlineOffset)) + 4));
                put(node, "mask-image", imageUrl(mask(geometry, margin)));
                put(node, "mask-clip", exactControl ? "border-box" : "no-clip");
                put(node, "mask-origin", "border-box");
                put(node, "mask-size", `${fmt(width + margin * 2)}px ${fmt(height + margin * 2)}px`);
                put(node, "mask-position", `${-margin}px ${-margin}px`);
                put(node, "mask-repeat", "no-repeat");
            }
            mark(node, clipping ? "clip" : "paint");
        }
        for (const [element, record] of records) {
            if (selected.has(element))
                continue;
            restore(element, record, true);
            resize.unobserve(element);
        }
        // Parse off-DOM, then commit once per element. Repeated setProperty calls
        // on attached nodes create needless style invalidations in large galleries.
        for (const [element, record] of records) {
            if (!record.pending.size)
                continue;
            const restoredStyle = element.style.cssText;
            const samePaint = record.appliedPaint?.size === record.pending.size && [...record.pending].every(([name, value]) => record.appliedPaint.get(name) === value);
            if (restoredStyle === record.restoredStyle && samePaint && record.appliedStyle !== undefined) {
                element.style.cssText = record.appliedStyle;
            }
            else {
                draftStyle.cssText = restoredStyle;
                for (const [name, value] of record.pending)
                    draftStyle.setProperty(name, value, "important");
                element.style.cssText = draftStyle.cssText;
            }
            record.restoredStyle = restoredStyle;
            record.appliedStyle = element.style.cssText;
            record.appliedPaint = new Map(record.pending);
            // Gecko may serialize colours differently after cssText is assigned to
            // the real node. Compare author edits with this actual applied value.
            for (const name of record.pending.keys())
                record.saved.get(name).applied = element.style.getPropertyValue(name);
            record.pending.clear();
        }
        // Backdrop/paint overrides can change an ancestor's fixed containing
        // block. Position only after every pending host style has been committed.
        for (const [element, record] of records)
            if (record.foreground?.isConnected && record.foreground.style.display !== "none")
                positionForeground(element, record.foreground);
        // Styles have been restored/replaced before unused URLs are revoked.
        // Keep at most 96 inactive SVGs for hover/theme reuse, plus live images.
        let inactive = [...imageUrls.keys()].filter(source => !usedImages.has(source));
        for (const source of inactive.slice(0, Math.max(0, inactive.length - 96))) {
            view.URL.revokeObjectURL(imageUrls.get(source));
            imageUrls.delete(source);
        }
        for (const target of activeExtentTransitions.keys()) {
            if (!target.isConnected || target.ownerDocument !== document)
                activeExtentTransitions.delete(target);
        }
        observe();
        if (activeExtentTransitions.size > 0)
            schedule();
    }
    function schedule() { if (!stopped && !frame)
        frame = view.requestAnimationFrame(update); }
    const events = ["pointerover", "pointerout", "pointerdown", "pointerup", "focusin", "focusout", "keydown", "keyup", "change", "input", "load", "DOMContentLoaded", "scroll", "beforetoggle", "toggle"];
    const interaction = (event) => {
        if (event.type === "scroll") {
            // Reposition only these few foreground frames synchronously; no full
            // gallery measurement is needed to keep native scrolling on its border.
            for (const [element, record] of records)
                if (record.foreground?.isConnected && record.foreground.style.display !== "none")
                    positionForeground(element, record.foreground);
        }
        for (let element = event.target instanceof view.Element ? event.target : null; element; element = element.parentElement) {
            const record = records.get(element);
            if (record)
                record.transitionDirty = true;
        }
        schedule();
    };
    for (const event of events)
        document.addEventListener(event, interaction, true);
    const animationStart = (event) => {
        // Size changes are already observed. Only an animated opt-in extent needs
        // a frame loop; spinners and self-written paint properties never start one.
        if (event instanceof view.TransitionEvent && ownedPaintProperty(event.propertyName) && records.has(event.target))
            return;
        if (event instanceof view.TransitionEvent && event.propertyName === "--xcss-server-corner-extent" && event.target instanceof view.Element) {
            activeExtentTransitions.set(event.target, (activeExtentTransitions.get(event.target) ?? 0) + 1);
        }
        schedule();
    };
    const animationEnd = (event) => {
        if (event instanceof view.TransitionEvent && ownedPaintProperty(event.propertyName) && records.has(event.target))
            return;
        if (event instanceof view.TransitionEvent && event.propertyName === "--xcss-server-corner-extent" && event.target instanceof view.Element) {
            const count = activeExtentTransitions.get(event.target) ?? 0;
            if (count > 1)
                activeExtentTransitions.set(event.target, count - 1);
            else
                activeExtentTransitions.delete(event.target);
        }
        schedule();
    };
    for (const name of ["transitionrun", "animationstart"])
        document.addEventListener(name, animationStart, true);
    for (const name of ["transitionend", "transitioncancel", "animationend", "animationcancel"])
        document.addEventListener(name, animationEnd, true);
    forced.addEventListener("change", schedule);
    colorScheme.addEventListener("change", schedule);
    document.fonts?.addEventListener("loadingdone", schedule);
    observe();
    schedule();
    const controller = {
        refresh: schedule,
        stop() {
            if (stopped)
                return;
            stopped = true;
            if (frame)
                view.cancelAnimationFrame(frame);
            activeExtentTransitions.clear();
            controllers.delete(document);
            observer.disconnect();
            resize.disconnect();
            for (const event of events)
                document.removeEventListener(event, interaction, true);
            for (const name of ["transitionrun", "animationstart"])
                document.removeEventListener(name, animationStart, true);
            for (const name of ["transitionend", "transitioncancel", "animationend", "animationcancel"])
                document.removeEventListener(name, animationEnd, true);
            forced.removeEventListener("change", schedule);
            colorScheme.removeEventListener("change", schedule);
            document.fonts?.removeEventListener("loadingdone", schedule);
            for (const [element, record] of records)
                restore(element, record, true);
            for (const url of imageUrls.values())
                view.URL.revokeObjectURL(url);
            imageUrls.clear();
            view.removeEventListener("pagehide", pagehide);
        },
    };
    controllers.set(document, controller);
    const pagehide = (event) => { if (!event.persisted)
        controller.stop(); };
    view.addEventListener("pagehide", pagehide);
    return controller;
}
