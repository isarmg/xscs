/** User-selected Mac mini enclosure fit: a non-rational degree-seven Bézier
 * corner, rotated four times and joined by straight sides. This is not an
 * identification of Apple's SwiftUI continuous or App Icon mask. */
export const SERVER_CORNER_PROFILE = "mac-mini-bezier7";
export const SERVER_CORNER_PHOTO_EXTENT = 0.31056899327095955;
/** Unit box, clockwise screen coordinates: (0,0) to (1,1). The source uses
 * y-up; only its y coordinate has been inverted. */
export const SERVER_CORNER_CONTROL_POINTS = Object.freeze([
    [0, 0], [0.3633595684909532, 0], [0.5804546942007682, 0], [0.7281206143651912, 0],
    [1, 0.27187938563480885], [1, 0.41954530579923177], [1, 0.6366404315090468], [1, 1],
].map(point => Object.freeze(point)));
const controls = SERVER_CORNER_CONTROL_POINTS.map(([x, y]) => ({ x, y }));
const derivative = (points) => points.slice(1).map((p, i) => ({ x: (points.length - 1) * (p.x - points[i].x), y: (points.length - 1) * (p.y - points[i].y) }));
const derivatives = [controls];
for (let order = 1; order <= 4; order++)
    derivatives.push(derivative(derivatives[order - 1]));
function finite(value, name) { if (!Number.isFinite(value))
    throw new RangeError(`${name} must be finite`); return value; }
function evaluate(points, t) {
    const row = points.map(p => ({ ...p }));
    for (let n = row.length - 1; n > 0; n--)
        for (let i = 0; i < n; i++)
            row[i] = { x: row[i].x * (1 - t) + row[i + 1].x * t, y: row[i].y * (1 - t) + row[i + 1].y * t };
    return row[0];
}
const cross = (a, b) => a.x * b.y - a.y * b.x;
const dot = (a, b) => a.x * b.x + a.y * b.y;
/** Analytic point/derivatives and arc-length curvature derivatives. Endpoint
 * κ=κ_s=0 gives G3 joins to straight lines; κ_ss is nonzero, so not G4. */
export function serverCornerProfile(t) {
    finite(t, "t");
    if (t < 0 || t > 1)
        throw new RangeError("t must be in [0, 1]");
    const [point, first, second, third, fourth] = derivatives.map(c => evaluate(c, t));
    const speed = Math.hypot(first.x, first.y), va = dot(first, second);
    const n = cross(first, second), nt = cross(first, third), ntt = cross(second, third) + cross(first, fourth);
    const kt = nt / speed ** 3 - 3 * n * va / speed ** 5;
    const ktt = ntt / speed ** 3 - 6 * nt * va / speed ** 5 - 3 * n * (dot(second, second) + dot(first, third)) / speed ** 5 + 15 * n * va ** 2 / speed ** 7;
    return Object.freeze({ ...point, first: Object.freeze(first), second: Object.freeze(second), third: Object.freeze(third), fourth: Object.freeze(fourth), speed,
        heading: Math.atan2(first.y, first.x), curvature: n / speed ** 3, curvatureDerivative: kt / speed,
        curvatureSecondDerivative: ktt / speed ** 2 - kt * va / speed ** 4 });
}
function splitVectors(points) {
    const rows = [points.map(p => ({ ...p }))];
    while (rows.at(-1).length > 1) {
        const row = rows.at(-1);
        rows.push(row.slice(1).map((p, i) => ({ x: (p.x + row[i].x) / 2, y: (p.y + row[i].y) / 2 })));
    }
    return [rows.map(row => row[0]), rows.map(row => row.at(-1)).reverse()];
}
function splitNumbers(points) {
    const rows = [[...points]];
    while (rows.at(-1).length > 1) {
        const row = rows.at(-1);
        rows.push(row.slice(1).map((p, i) => (p + row[i]) / 2));
    }
    return [rows.map(row => row[0]), rows.map(row => row.at(-1)).reverse()];
}
function binomial(n, k) { let result = 1; for (let i = 1; i <= k; i++)
    result = result * (n - i + 1) / i; return result; }
function product(a, b) {
    const n = a.length - 1, m = b.length - 1;
    return Array.from({ length: n + m + 1 }, (_, k) => { let result = 0; for (let i = Math.max(0, k - m); i <= Math.min(n, k); i++)
        result += a[i] * b[k - i] * binomial(n, i) * binomial(m, k - i) / binomial(n + m, k); return result; });
}
function minimumSpeed(points) {
    const distance = (axis) => { const lo = Math.min(...points.map(p => p[axis])), hi = Math.max(...points.map(p => p[axis])); return lo > 0 ? lo : hi < 0 ? -hi : 0; };
    return Math.hypot(distance("x"), distance("y"));
}
/** Polynomial convex-hull certificate, not a sampled curvature maximum.
 * cross(B',B'') has nonnegative degree-11 Bernstein coefficients, proving
 * convexity. 512 dyadic interval hulls bound its maximum and minimum speed.
 * A relative 1e-10 margin covers IEEE arithmetic, giving a conservative
 * minimum radius used to reject inward offsets before possible cusps. */
function certifyProfile() {
    const v = derivatives[1], a = derivatives[2], xy = product(v.map(p => p.x), a.map(p => p.y)), yx = product(v.map(p => p.y), a.map(p => p.x));
    const numerator = xy.map((value, i) => value - yx[i]);
    if (Math.min(...numerator) < -1e-12)
        throw new Error("Mac mini profile convexity certificate failed");
    let parts = [{ velocity: v, numerator }];
    for (let depth = 0; depth < 9; depth++)
        parts = parts.flatMap(part => { const [vl, vr] = splitVectors(part.velocity), [nl, nr] = splitNumbers(part.numerator); return [{ velocity: vl, numerator: nl }, { velocity: vr, numerator: nr }]; });
    let maxCurvature = 0, minSpeed = Infinity;
    for (const part of parts) {
        const speed = minimumSpeed(part.velocity);
        minSpeed = Math.min(minSpeed, speed);
        maxCurvature = Math.max(maxCurvature, Math.max(...part.numerator) / speed ** 3);
    }
    maxCurvature *= 1 + 1e-10;
    minSpeed *= 1 - 1e-10;
    return Object.freeze({ intervals: parts.length, minimumSpeed: minSpeed, maximumCurvature: maxCurvature, minimumRadius: 1 / maxCurvature, convex: true, arithmeticRelativeMargin: 1e-10 });
}
export const SERVER_CORNER_PROFILE_BOUNDS = certifyProfile();
const fmt = (value) => Number(value.toFixed(6)).toString();
const pair = (point) => `${fmt(point.x)} ${fmt(point.y)}`;
const freezePoints = (points) => Object.freeze(points.map(point => Object.freeze(point)));
const geometryCache = new Map();
const sampleCache = new Map();
const pathCache = new WeakMap();
const geometryTolerance = new WeakMap();
function cacheSet(cache, key, value, limit = 96) { cache.set(key, value); if (cache.size > limit)
    cache.delete(cache.keys().next().value); return value; }
function cacheGet(cache, key) { const value = cache.get(key); if (value !== undefined) {
    cache.delete(key);
    cache.set(key, value);
} return value; }
function profilePoint(t, offset) {
    const q = serverCornerProfile(t);
    return { x: q.x - offset * Math.sin(q.heading), y: q.y + offset * Math.cos(q.heading) - offset, heading: q.heading, parameter: t };
}
/** Q=B+D*n, with T=B'/|B'| and inward unit normal n. Bounds:
 * |T''| <= J/v_min+3*A²/v_min²; |Q''| <= A+|D|*|T''|.
 * Parameter interval width h gives interpolation deviation <= h²*sup|Q''|/8.
 * Derivative Bézier hulls supply every interval bound; no dense-grid guess.
 * SVG uses chords, which are not themselves mathematically G3. */
function sampleProfile(offset, budget) {
    const key = `${offset},${budget}`, cached = cacheGet(sampleCache, key);
    if (cached)
        return cached;
    const points = [profilePoint(0, offset)];
    let bound = 0;
    const visit = (v, a, j, start, end, depth) => {
        const speed = minimumSpeed(v), acceleration = Math.max(...a.map(p => Math.hypot(p.x, p.y))), jerk = Math.max(...j.map(p => Math.hypot(p.x, p.y)));
        const second = acceleration + Math.abs(offset) * (jerk / speed + 3 * acceleration ** 2 / speed ** 2);
        const error = second * (end - start) ** 2 / 8 * (1 + 1e-10);
        if (Number.isFinite(error) && error <= budget) {
            bound = Math.max(bound, error);
            points.push(profilePoint(end, offset));
            return;
        }
        if (depth === 24)
            throw new RangeError("Corner sampling cannot meet the requested budget");
        const [vl, vr] = splitVectors(v), [al, ar] = splitVectors(a), [jl, jr] = splitVectors(j), middle = (start + end) / 2;
        visit(vl, al, jl, start, middle, depth + 1);
        visit(vr, ar, jr, middle, end, depth + 1);
    };
    visit(derivatives[1], derivatives[2], derivatives[3], 0, 1, 0);
    return cacheSet(sampleCache, key, Object.freeze({ points: freezePoints(points), bound }));
}
function sourceFor(extent) { return Object.freeze({ extent, radius: extent * SERVER_CORNER_PROFILE_BOUNDS.minimumRadius, offset: 0 }); }
function regular(source) { return source.extent > 0 && source.offset < source.radius - Math.max(source.extent * Number.EPSILON * 16, source.radius * 1e-10); }
function localCurve(source, tolerance) {
    const extent = source.extent - source.offset;
    if (source.extent === 0)
        return { extent: 0, radius: 0, points: [{ x: 0, y: 0, heading: 0 }, { x: 0, y: 0, heading: Math.PI / 2 }], bound: 0 };
    if (!regular(source))
        throw new RangeError("Corner offset is not certified regular");
    const size = source.extent, allowance = 2e-6 + size * 1e-10;
    if (allowance >= tolerance / 2)
        throw new RangeError("Tolerance is smaller than arithmetic/serialization allowance");
    const sampled = sampleProfile(source.offset / source.extent, tolerance / (2 * size));
    return { extent, radius: source.radius - source.offset, points: sampled.points.map(q => ({ ...q, x: q.x * size, y: q.y * size })), bound: sampled.bound * size + allowance };
}
function transform(q, index, width, height, extent) {
    if (index === 0)
        return { ...q, x: width - extent + q.x, y: q.y };
    if (index === 1)
        return { ...q, x: width - q.y, y: height - extent + q.x, heading: q.heading + Math.PI / 2 };
    if (index === 2)
        return { ...q, x: extent - q.x, y: height - q.y, heading: q.heading + Math.PI };
    return { ...q, x: q.y, y: extent - q.x, heading: q.heading + 3 * Math.PI / 2 };
}
function assemble(width, height, source, tolerance, canonical) {
    const local = localCurve(source, tolerance);
    let errorBound = local.bound;
    const corners = Array.from({ length: 4 }, (_, index) => {
        return Object.freeze({ extent: local.extent, radius: local.radius, source, points: freezePoints(local.points.map(q => transform(q, index, width, height, local.extent))) });
    });
    // Translation to a large border-box origin also has floating-point error;
    // sharp corners still need a decimal serialization allowance.
    errorBound += 2e-6 + Math.max(width, height) * Number.EPSILON * 16;
    if (errorBound > tolerance)
        throw new RangeError("Corner cannot meet the requested coordinate budget");
    const points = freezePoints([{ x: corners[3].extent, y: 0, heading: 0 }, ...corners.flatMap(corner => [...corner.points])]);
    const path = points.map((q, i) => `${i ? "L" : "M"}${pair(q)}`).join("") + "Z";
    const result = Object.freeze({ width, height, extent: local.extent, radius: local.radius, path, points,
        corners: Object.freeze(corners), canonical, errorBound });
    geometryTolerance.set(result, tolerance);
    return result;
}
/** Four identical physical corner boxes; clamping to half the short side
 * prevents overlap without anisotropically stretching the source profile. */
export function createServerCornerGeometry(options) {
    const width = finite(options.width, "width"), height = finite(options.height, "height"), requested = finite(options.extent, "extent"), tolerance = finite(options.tolerance ?? .05, "tolerance");
    if (width <= 0 || height <= 0 || requested < 0 || tolerance <= 0)
        throw new RangeError("Invalid corner dimensions or tolerance");
    const extent = Math.min(requested, width / 2, height / 2);
    const key = [width, height, extent, tolerance].join(","), cached = cacheGet(geometryCache, key);
    if (cached)
        return cached;
    return cacheSet(geometryCache, key, assemble(width, height, sourceFor(extent), tolerance, true));
}
/** Outward shadow/outline spread. Resample the analytic parallel profile at
 * its own derivative bound; shifting old coarse chords would not retain the
 * original error budget. No affine stretching of the whole silhouette. */
export function offsetServerCornerPath(geometry, offset) {
    finite(offset, "offset");
    if (offset === 0)
        return geometry.path;
    if (geometry.width + offset * 2 <= 0 || geometry.height + offset * 2 <= 0)
        throw new RangeError("Inset leaves no positive shadow dimensions");
    if (offset < 0 && geometry.radius > 0 && offset <= -geometry.radius)
        throw new RangeError("Inset exceeds certified minimum curvature radius");
    let cache = pathCache.get(geometry);
    if (!cache) {
        cache = new Map();
        pathCache.set(geometry, cache);
    }
    const cached = cacheGet(cache, offset);
    if (cached)
        return cached;
    const tolerance = geometryTolerance.get(geometry) ?? .05;
    const source = geometry.corners[0].source;
    const expanded = assemble(geometry.width + offset * 2, geometry.height + offset * 2, source.extent === 0 ? sourceFor(0) : Object.freeze({ ...source, offset: source.offset - offset }), tolerance, false);
    const points = expanded.points.map(q => ({ x: q.x - offset, y: q.y - offset }));
    return cacheSet(cache, offset, points.map((q, i) => `${i ? "L" : "M"}${pair(q)}`).join("") + "Z", 16);
}
