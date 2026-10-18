//! The selected non-rational degree-seven Mac mini enclosure profile.
//! Derivative hulls certify curvature and each adaptive interpolation budget;
//! the emitted SVG chords themselves are not mathematically G3 curves.
use serde::Serialize;
use std::{cell::RefCell, collections::VecDeque, f64::consts::PI, rc::Rc};

pub const PROFILE: &str = "mac-mini-bezier7";
pub const PHOTO_EXTENT: f64 = 0.31056899327095955;
pub const CONTROL_POINTS: [[f64; 2]; 8] = [
    [0.0, 0.0],
    [0.3633595684909532, 0.0],
    [0.5804546942007682, 0.0],
    [0.7281206143651912, 0.0],
    [1.0, 0.27187938563480885],
    [1.0, 0.41954530579923177],
    [1.0, 0.6366404315090468],
    [1.0, 1.0],
];

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vector {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub x: f64,
    pub y: f64,
    pub first: Vector,
    pub second: Vector,
    pub third: Vector,
    pub fourth: Vector,
    pub speed: f64,
    pub heading: f64,
    pub curvature: f64,
    pub curvature_derivative: f64,
    pub curvature_second_derivative: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub intervals: usize,
    pub minimum_speed: f64,
    pub maximum_curvature: f64,
    pub minimum_radius: f64,
    pub convex: bool,
    pub arithmetic_relative_margin: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub extent: f64,
    pub radius: f64,
    pub offset: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Curve {
    pub extent: f64,
    pub radius: f64,
    pub points: Vec<Point>,
    pub source: Source,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub extent: f64,
    pub radius: f64,
    pub path: String,
    pub points: Vec<Point>,
    pub corners: Vec<Curve>,
    pub canonical: bool,
    pub error_bound: f64,
    #[serde(skip)]
    pub tolerance: f64,
    #[serde(skip)]
    offset_paths: RefCell<Cache<u64, String>>,
}

#[derive(Debug)]
struct Cache<K, V>(VecDeque<(K, V)>);

impl<K: PartialEq, V: Clone> Cache<K, V> {
    fn new() -> Self {
        Self(VecDeque::new())
    }

    fn get(&mut self, key: &K) -> Option<V> {
        let index = self.0.iter().position(|(item, _)| item == key)?;
        let entry = self.0.remove(index)?;
        let value = entry.1.clone();
        self.0.push_back(entry);
        Some(value)
    }

    fn set(&mut self, key: K, value: V, limit: usize) -> V {
        if let Some(index) = self.0.iter().position(|(item, _)| item == &key) {
            self.0.remove(index);
        }
        self.0.push_back((key, value.clone()));
        if self.0.len() > limit {
            self.0.pop_front();
        }
        value
    }
}

#[derive(Clone)]
struct Sample {
    points: Vec<Point>,
    bound: f64,
}

thread_local! {
    static DERIVATIVES: [Vec<Vector>; 5] = {
        let mut result: [Vec<Vector>; 5] = std::array::from_fn(|_| Vec::new());
        result[0] = CONTROL_POINTS.map(|[x, y]| Vector { x, y }).to_vec();
        for order in 1..=4 {
            result[order] = derivative(&result[order - 1]);
        }
        result
    };
    static BOUNDS: Bounds = certify_profile();
    static GEOMETRIES: RefCell<Cache<[u64; 4], Rc<Geometry>>> = RefCell::new(Cache::new());
    static SAMPLES: RefCell<Cache<[u64; 2], Sample>> = RefCell::new(Cache::new());
}

fn key(value: f64) -> u64 {
    // JavaScript Map keys and its original string keys identify -0 with +0.
    if value == 0.0 { 0 } else { value.to_bits() }
}

fn finite(value: f64, name: &str) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{name} must be finite"))
    }
}

fn derivative(points: &[Vector]) -> Vec<Vector> {
    let degree = (points.len() - 1) as f64;
    points
        .windows(2)
        .map(|pair| Vector {
            x: degree * (pair[1].x - pair[0].x),
            y: degree * (pair[1].y - pair[0].y),
        })
        .collect()
}

fn evaluate(points: &[Vector], t: f64) -> Vector {
    let mut row = points.to_vec();
    for length in (1..row.len()).rev() {
        for index in 0..length {
            row[index] = Vector {
                x: row[index].x * (1.0 - t) + row[index + 1].x * t,
                y: row[index].y * (1.0 - t) + row[index + 1].y * t,
            };
        }
    }
    row[0]
}

fn cross(a: Vector, b: Vector) -> f64 {
    a.x * b.y - a.y * b.x
}

fn dot(a: Vector, b: Vector) -> f64 {
    a.x * b.x + a.y * b.y
}

/// Analytic position, derivatives, curvature, and arc-length derivatives.
pub fn profile(t: f64) -> Result<Profile, String> {
    finite(t, "t")?;
    if !(0.0..=1.0).contains(&t) {
        return Err("t must be in [0, 1]".into());
    }
    let [point, first, second, third, fourth] =
        DERIVATIVES.with(|derivatives| derivatives.each_ref().map(|row| evaluate(row, t)));
    let speed = first.x.hypot(first.y);
    let va = dot(first, second);
    let n = cross(first, second);
    let nt = cross(first, third);
    let ntt = cross(second, third) + cross(first, fourth);
    let kt = nt / speed.powi(3) - 3.0 * n * va / speed.powi(5);
    let ktt = ntt / speed.powi(3)
        - 6.0 * nt * va / speed.powi(5)
        - 3.0 * n * (dot(second, second) + dot(first, third)) / speed.powi(5)
        + 15.0 * n * va.powi(2) / speed.powi(7);
    Ok(Profile {
        x: point.x,
        y: point.y,
        first,
        second,
        third,
        fourth,
        speed,
        heading: first.y.atan2(first.x),
        curvature: n / speed.powi(3),
        curvature_derivative: kt / speed,
        curvature_second_derivative: ktt / speed.powi(2) - kt * va / speed.powi(4),
    })
}

fn split_vectors(points: &[Vector]) -> (Vec<Vector>, Vec<Vector>) {
    let mut row = points.to_vec();
    let mut left = vec![row[0]];
    let mut right = vec![row[row.len() - 1]];
    while row.len() > 1 {
        row = row
            .windows(2)
            .map(|pair| Vector {
                x: (pair[0].x + pair[1].x) / 2.0,
                y: (pair[0].y + pair[1].y) / 2.0,
            })
            .collect();
        left.push(row[0]);
        right.push(row[row.len() - 1]);
    }
    right.reverse();
    (left, right)
}

fn split_numbers(points: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut row = points.to_vec();
    let mut left = vec![row[0]];
    let mut right = vec![row[row.len() - 1]];
    while row.len() > 1 {
        row = row
            .windows(2)
            .map(|pair| (pair[0] + pair[1]) / 2.0)
            .collect();
        left.push(row[0]);
        right.push(row[row.len() - 1]);
    }
    right.reverse();
    (left, right)
}

fn binomial(n: usize, k: usize) -> f64 {
    let mut result = 1.0;
    for i in 1..=k {
        result = result * (n - i + 1) as f64 / i as f64;
    }
    result
}

fn product(a: &[f64], b: &[f64]) -> Vec<f64> {
    let n = a.len() - 1;
    let m = b.len() - 1;
    (0..=n + m)
        .map(|k| {
            let mut result = 0.0;
            for i in k.saturating_sub(m)..=n.min(k) {
                result +=
                    a[i] * b[k - i] * binomial(n, i) * binomial(m, k - i) / binomial(n + m, k);
            }
            result
        })
        .collect()
}

fn minimum_speed(points: &[Vector]) -> f64 {
    let distance = |axis: fn(&Vector) -> f64| {
        let low = points.iter().map(axis).fold(f64::INFINITY, f64::min);
        let high = points.iter().map(axis).fold(f64::NEG_INFINITY, f64::max);
        if low > 0.0 {
            low
        } else if high < 0.0 {
            -high
        } else {
            0.0
        }
    };
    distance(|q| q.x).hypot(distance(|q| q.y))
}

/// Polynomial convex-hull certificate over 512 dyadic intervals, rather than
/// a sampled maximum. The relative margin covers floating-point arithmetic.
fn certify_profile() -> Bounds {
    let (velocity, numerator) = DERIVATIVES.with(|derivatives| {
        let v = &derivatives[1];
        let a = &derivatives[2];
        let vx: Vec<_> = v.iter().map(|p| p.x).collect();
        let vy: Vec<_> = v.iter().map(|p| p.y).collect();
        let ax: Vec<_> = a.iter().map(|p| p.x).collect();
        let ay: Vec<_> = a.iter().map(|p| p.y).collect();
        let numerator: Vec<_> = product(&vx, &ay)
            .into_iter()
            .zip(product(&vy, &ax))
            .map(|(x, y)| x - y)
            .collect();
        (v.clone(), numerator)
    });
    assert!(
        numerator.iter().all(|value| *value >= -1e-12),
        "Mac mini profile convexity certificate failed"
    );
    let mut parts = vec![(velocity, numerator)];
    for _ in 0..9 {
        parts = parts
            .into_iter()
            .flat_map(|(velocity, numerator)| {
                let (vl, vr) = split_vectors(&velocity);
                let (nl, nr) = split_numbers(&numerator);
                [(vl, nl), (vr, nr)]
            })
            .collect();
    }
    let mut maximum_curvature: f64 = 0.0;
    let mut minimum: f64 = f64::INFINITY;
    for (velocity, numerator) in &parts {
        let speed = minimum_speed(velocity);
        minimum = minimum.min(speed);
        maximum_curvature = maximum_curvature
            .max(numerator.iter().copied().fold(f64::NEG_INFINITY, f64::max) / speed.powi(3));
    }
    maximum_curvature *= 1.0 + 1e-10;
    minimum *= 1.0 - 1e-10;
    Bounds {
        intervals: parts.len(),
        minimum_speed: minimum,
        maximum_curvature,
        minimum_radius: 1.0 / maximum_curvature,
        convex: true,
        arithmetic_relative_margin: 1e-10,
    }
}

pub fn bounds() -> Bounds {
    BOUNDS.with(|value| *value)
}

/// Match `Number(value.toFixed(6)).toString()` without a JavaScript runtime.
/// Integer arithmetic rounds the exact binary input, including decimal ties;
/// multiplying by 1e6 first or Rust's ties-to-even formatting would differ.
pub fn fmt(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        }
        .into();
    }
    let absolute = value.abs();
    if absolute >= 1e21 {
        let scientific = format!("{value:e}");
        let (mantissa, exponent) = scientific.split_once('e').expect("scientific notation");
        let exponent: i32 = exponent.parse().expect("decimal exponent");
        return format!("{mantissa}e{exponent:+}");
    }
    let bits = absolute.to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    let (mantissa, exponent) = if encoded_exponent == 0 {
        (bits & ((1_u64 << 52) - 1), -1074)
    } else {
        (
            (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52),
            encoded_exponent - 1023 - 52,
        )
    };
    let numerator = u128::from(mantissa) * 1_000_000;
    let rounded = if exponent >= 0 {
        numerator << exponent as u32
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let quotient = numerator >> shift;
            let remainder = numerator - (quotient << shift);
            quotient + u128::from(remainder >= (1_u128 << (shift - 1)))
        }
    };
    if rounded == 0 {
        return "0".into();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let integer = rounded / 1_000_000;
    let fraction = rounded % 1_000_000;
    let decimal = if fraction == 0 {
        format!("{sign}{integer}")
    } else {
        let fraction = format!("{fraction:06}");
        format!("{sign}{integer}.{}", fraction.trim_end_matches('0'))
    };
    // toFixed returns a decimal string, but the original Number conversion
    // then prints the shortest round-tripping number (relevant above 2^53).
    decimal.parse::<f64>().expect("rounded decimal").to_string()
}

fn path(points: &[Point], offset: f64) -> String {
    let mut result = String::new();
    for (index, q) in points.iter().enumerate() {
        result.push(if index == 0 { 'M' } else { 'L' });
        result.push_str(&fmt(q.x - offset));
        result.push(' ');
        result.push_str(&fmt(q.y - offset));
    }
    result.push('Z');
    result
}

fn profile_point(t: f64, offset: f64) -> Point {
    let q = profile(t).expect("sampling uses a unit-interval parameter");
    Point {
        x: q.x - offset * q.heading.sin(),
        y: q.y + offset * q.heading.cos() - offset,
        heading: q.heading,
        parameter: Some(t),
    }
}

struct Sampler {
    offset: f64,
    budget: f64,
    points: Vec<Point>,
    bound: f64,
}

impl Sampler {
    fn visit(
        &mut self,
        velocity: &[Vector],
        acceleration: &[Vector],
        jerk: &[Vector],
        interval: [f64; 2],
        depth: usize,
    ) -> Result<(), String> {
        let [start, end] = interval;
        let speed = minimum_speed(velocity);
        let acceleration_bound = acceleration
            .iter()
            .map(|q| q.x.hypot(q.y))
            .fold(f64::NEG_INFINITY, f64::max);
        let jerk_bound = jerk
            .iter()
            .map(|q| q.x.hypot(q.y))
            .fold(f64::NEG_INFINITY, f64::max);
        let second = acceleration_bound
            + self.offset.abs()
                * (jerk_bound / speed + 3.0 * acceleration_bound.powi(2) / speed.powi(2));
        let error = second * (end - start).powi(2) / 8.0 * (1.0 + 1e-10);
        if error.is_finite() && error <= self.budget {
            self.bound = self.bound.max(error);
            self.points.push(profile_point(end, self.offset));
            return Ok(());
        }
        if depth == 24 {
            return Err("Corner sampling cannot meet the requested budget".into());
        }
        let (vl, vr) = split_vectors(velocity);
        let (al, ar) = split_vectors(acceleration);
        let (jl, jr) = split_vectors(jerk);
        let middle = (start + end) / 2.0;
        self.visit(&vl, &al, &jl, [start, middle], depth + 1)?;
        self.visit(&vr, &ar, &jr, [middle, end], depth + 1)
    }
}

fn sample_profile(offset: f64, budget: f64) -> Result<Sample, String> {
    let key = [key(offset), key(budget)];
    if let Some(cached) = SAMPLES.with(|cache| cache.borrow_mut().get(&key)) {
        return Ok(cached);
    }
    let mut sampler = Sampler {
        offset,
        budget,
        points: vec![profile_point(0.0, offset)],
        bound: 0.0,
    };
    DERIVATIVES.with(|derivatives| {
        sampler.visit(
            &derivatives[1],
            &derivatives[2],
            &derivatives[3],
            [0.0, 1.0],
            0,
        )
    })?;
    let sampled = Sample {
        points: sampler.points,
        bound: sampler.bound,
    };
    Ok(SAMPLES.with(|cache| cache.borrow_mut().set(key, sampled, 96)))
}

fn source_for(extent: f64) -> Source {
    Source {
        extent,
        radius: extent * bounds().minimum_radius,
        offset: 0.0,
    }
}

fn regular(source: Source) -> bool {
    source.extent > 0.0
        && source.offset
            < source.radius - (source.extent * f64::EPSILON * 16.0).max(source.radius * 1e-10)
}

struct LocalCurve {
    extent: f64,
    radius: f64,
    points: Vec<Point>,
    bound: f64,
}

fn local_curve(source: Source, tolerance: f64) -> Result<LocalCurve, String> {
    if source.extent == 0.0 {
        return Ok(LocalCurve {
            extent: 0.0,
            radius: 0.0,
            points: vec![
                Point {
                    x: 0.0,
                    y: 0.0,
                    heading: 0.0,
                    parameter: None,
                },
                Point {
                    x: 0.0,
                    y: 0.0,
                    heading: PI / 2.0,
                    parameter: None,
                },
            ],
            bound: 0.0,
        });
    }
    if !regular(source) {
        return Err("Corner offset is not certified regular".into());
    }
    let size = source.extent;
    let allowance = 2e-6 + size * 1e-10;
    if allowance >= tolerance / 2.0 {
        return Err("Tolerance is smaller than arithmetic/serialization allowance".into());
    }
    let sampled = sample_profile(source.offset / size, tolerance / (2.0 * size))?;
    Ok(LocalCurve {
        extent: source.extent - source.offset,
        radius: source.radius - source.offset,
        points: sampled
            .points
            .iter()
            .map(|q| Point {
                x: q.x * size,
                y: q.y * size,
                ..*q
            })
            .collect(),
        bound: sampled.bound * size + allowance,
    })
}

fn transform(q: Point, index: usize, width: f64, height: f64, extent: f64) -> Point {
    match index {
        0 => Point {
            x: width - extent + q.x,
            ..q
        },
        1 => Point {
            x: width - q.y,
            y: height - extent + q.x,
            heading: q.heading + PI / 2.0,
            ..q
        },
        2 => Point {
            x: extent - q.x,
            y: height - q.y,
            heading: q.heading + PI,
            ..q
        },
        _ => Point {
            x: q.y,
            y: extent - q.x,
            heading: q.heading + 3.0 * PI / 2.0,
            ..q
        },
    }
}

fn assemble(
    width: f64,
    height: f64,
    source: Source,
    tolerance: f64,
    canonical: bool,
) -> Result<Geometry, String> {
    let local = local_curve(source, tolerance)?;
    let error_bound = local.bound + (2e-6 + width.max(height) * f64::EPSILON * 16.0);
    if error_bound > tolerance {
        return Err("Corner cannot meet the requested coordinate budget".into());
    }
    let corners: Vec<_> = (0..4)
        .map(|index| Curve {
            extent: local.extent,
            radius: local.radius,
            source,
            points: local
                .points
                .iter()
                .map(|q| transform(*q, index, width, height, local.extent))
                .collect(),
        })
        .collect();
    let mut points = vec![Point {
        x: corners[3].extent,
        y: 0.0,
        heading: 0.0,
        parameter: None,
    }];
    points.extend(
        corners
            .iter()
            .flat_map(|corner| corner.points.iter().copied()),
    );
    Ok(Geometry {
        width,
        height,
        extent: local.extent,
        radius: local.radius,
        path: path(&points, 0.0),
        points,
        corners,
        canonical,
        error_bound,
        tolerance,
        offset_paths: RefCell::new(Cache::new()),
    })
}

/// Clamp identical physical corner boxes to half the short side, retaining
/// the original profile without stretching one axis independently.
pub fn create(
    width: f64,
    height: f64,
    extent: f64,
    tolerance: f64,
) -> Result<Rc<Geometry>, String> {
    finite(width, "width")?;
    finite(height, "height")?;
    finite(extent, "extent")?;
    finite(tolerance, "tolerance")?;
    if width <= 0.0 || height <= 0.0 || extent < 0.0 || tolerance <= 0.0 {
        return Err("Invalid corner dimensions or tolerance".into());
    }
    let extent = extent.min(width / 2.0).min(height / 2.0);
    let key = [key(width), key(height), key(extent), key(tolerance)];
    if let Some(cached) = GEOMETRIES.with(|cache| cache.borrow_mut().get(&key)) {
        return Ok(cached);
    }
    let result = Rc::new(assemble(
        width,
        height,
        source_for(extent),
        tolerance,
        true,
    )?);
    Ok(GEOMETRIES.with(|cache| cache.borrow_mut().set(key, result, 96)))
}

/// Resample the analytic parallel profile with its own derivative bound;
/// translating previously sampled coarse chords would lose the error budget.
pub fn offset_path(geometry: &Geometry, offset: f64) -> Result<String, String> {
    finite(offset, "offset")?;
    if offset == 0.0 {
        return Ok(geometry.path.clone());
    }
    if geometry.width + offset * 2.0 <= 0.0 || geometry.height + offset * 2.0 <= 0.0 {
        return Err("Inset leaves no positive shadow dimensions".into());
    }
    if offset < 0.0 && geometry.radius > 0.0 && offset <= -geometry.radius {
        return Err("Inset exceeds certified minimum curvature radius".into());
    }
    if let Some(cached) = geometry.offset_paths.borrow_mut().get(&key(offset)) {
        return Ok(cached);
    }
    let source = geometry.corners[0].source;
    let source = if source.extent == 0.0 {
        source_for(0.0)
    } else {
        Source {
            offset: source.offset - offset,
            ..source
        }
    };
    let expanded = assemble(
        geometry.width + offset * 2.0,
        geometry.height + offset * 2.0,
        source,
        geometry.tolerance,
        false,
    )?;
    let result = path(&expanded.points, offset);
    Ok(geometry
        .offset_paths
        .borrow_mut()
        .set(key(offset), result, 16))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }

    // Independent Bernstein summation, rather than production de Casteljau.
    fn polynomial(controls: &[[f64; 2]], t: f64) -> Vector {
        let n = controls.len() - 1;
        let mut result = Vector { x: 0.0, y: 0.0 };
        for (i, [x, y]) in controls.iter().enumerate() {
            let weight = binomial(n, i) * t.powi(i as i32) * (1.0 - t).powi((n - i) as i32);
            result.x += x * weight;
            result.y += y * weight;
        }
        result
    }

    fn reference(t: f64, source: Source) -> Vector {
        let controls: Vec<_> = CONTROL_POINTS
            .windows(2)
            .map(|pair| {
                [
                    7.0 * (pair[1][0] - pair[0][0]),
                    7.0 * (pair[1][1] - pair[0][1]),
                ]
            })
            .collect();
        let q = polynomial(&CONTROL_POINTS, t);
        let v = polynomial(&controls, t);
        let speed = v.x.hypot(v.y);
        Vector {
            x: q.x * source.extent - source.offset * v.y / speed,
            y: q.y * source.extent + source.offset * (v.x / speed - 1.0),
        }
    }

    fn local(q: Point, corner: &Curve, geometry: &Geometry, index: usize) -> Vector {
        match index {
            0 => Vector {
                x: q.x - (geometry.width - corner.extent),
                y: q.y,
            },
            1 => Vector {
                x: q.y - (geometry.height - corner.extent),
                y: geometry.width - q.x,
            },
            2 => Vector {
                x: corner.extent - q.x,
                y: geometry.height - q.y,
            },
            _ => Vector {
                x: corner.extent - q.y,
                y: q.x,
            },
        }
    }

    fn assert_convex(geometry: &Geometry) {
        let mut distinct: Vec<Point> = Vec::new();
        for point in &geometry.points {
            if distinct
                .last()
                .is_none_or(|last| (point.x - last.x).hypot(point.y - last.y) > 1e-8)
            {
                distinct.push(*point);
            }
        }
        if (distinct[0].x - distinct[distinct.len() - 1].x)
            .hypot(distinct[0].y - distinct[distinct.len() - 1].y)
            < 1e-8
        {
            distinct.pop();
        }
        for index in 0..distinct.len() {
            let a = distinct[index];
            let b = distinct[(index + 1) % distinct.len()];
            let c = distinct[(index + 2) % distinct.len()];
            assert!((b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) >= -1e-7);
        }
        for q in &geometry.points {
            assert!(q.x >= -1e-8 && q.x <= geometry.width + 1e-8);
            assert!(q.y >= -1e-8 && q.y <= geometry.height + 1e-8);
        }
    }

    fn check_error(geometry: &Geometry) {
        let mut observed: f64 = 0.0;
        for (index, corner) in geometry.corners.iter().enumerate() {
            if corner.extent == 0.0 {
                continue;
            }
            for pair in corner.points.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                let rounded = |p: Point| Point {
                    x: fmt(p.x).parse().unwrap(),
                    y: fmt(p.y).parse().unwrap(),
                    ..p
                };
                let al = local(rounded(a), corner, geometry, index);
                let bl = local(rounded(b), corner, geometry, index);
                for j in 1..16 {
                    let fraction = f64::from(j) / 16.0;
                    let t = a.parameter.unwrap()
                        + (b.parameter.unwrap() - a.parameter.unwrap()) * fraction;
                    let q = reference(t, corner.source);
                    observed = observed.max(
                        (q.x - al.x * (1.0 - fraction) - bl.x * fraction)
                            .hypot(q.y - al.y * (1.0 - fraction) - bl.y * fraction),
                    );
                }
            }
        }
        assert!(
            observed <= geometry.error_bound,
            "observed {observed} > {}",
            geometry.error_bound
        );
    }

    #[test]
    fn source_controls_proportions_and_independent_polynomial_agree() {
        assert_eq!(PROFILE, "mac-mini-bezier7");
        assert_eq!(PHOTO_EXTENT, 0.31056899327095955);
        assert_eq!(CONTROL_POINTS[3], [0.7281206143651912, 0.0]);
        let diagonal = profile(0.5).unwrap();
        close(diagonal.x, 0.8141975551596445, 1e-10);
        close(diagonal.y, 0.1858024448403555, 1e-10);
        close(diagonal.heading, PI / 4.0, 1e-10);
        close(diagonal.curvature, 1.7600990604903817, 1e-10);
        for i in 0..=100 {
            let t = f64::from(i) / 100.0;
            let q = profile(t).unwrap();
            let independent = polynomial(&CONTROL_POINTS, t);
            close(q.x, independent.x, 1e-10);
            close(q.y, independent.y, 1e-10);
        }
        let geometry = create(127.0, 127.0, 127.0 * PHOTO_EXTENT, 0.05).unwrap();
        close(geometry.extent, 39.44226214541186, 1e-10);
        close(
            geometry.width - 2.0 * geometry.extent,
            48.11547570917629,
            1e-10,
        );
    }

    #[test]
    fn analytic_endpoints_join_lines_at_g3_not_g4() {
        let start = profile(0.0).unwrap();
        let end = profile(1.0).unwrap();
        close(start.x, 0.0, 1e-10);
        close(start.y, 0.0, 1e-10);
        close(end.x, 1.0, 1e-10);
        close(end.y, 1.0, 1e-10);
        close(start.heading, 0.0, 1e-10);
        close(end.heading, PI / 2.0, 1e-10);
        for q in [&start, &end] {
            close(q.curvature, 0.0, 1e-10);
            close(q.curvature_derivative, 0.0, 1e-10);
            assert!(q.curvature_second_derivative > 5.4 && q.curvature_second_derivative < 5.5);
            assert!(q.speed > 0.0);
        }
        assert!(start.fourth.y > 0.0 && end.fourth.x < 0.0);
    }

    #[test]
    fn polynomial_hulls_certify_speed_curvature_and_offset_radius() {
        let bound = bounds();
        assert_eq!(bound.intervals, 512);
        assert!(bound.convex);
        assert!(bound.minimum_radius > 0.563 && bound.minimum_radius < 0.565);
        let mut maximum: f64 = 0.0;
        for i in 0..=5000 {
            let q = profile(f64::from(i) / 5000.0).unwrap();
            assert!(q.speed >= bound.minimum_speed);
            assert!(q.curvature >= -1e-12 && q.curvature <= bound.maximum_curvature);
            maximum = maximum.max(q.curvature);
        }
        assert!(bound.maximum_curvature / maximum < 1.01);
    }

    #[test]
    fn corner_boxes_are_convex_across_rectangles_and_small_extents() {
        for [width, height] in [
            [12.0, 48.0],
            [48.0, 12.0],
            [320.0, 216.0],
            [216.0, 144.0],
            [1024.0, 1024.0],
        ] {
            for extent in [0.0, 1e-12, 0.01, 27.0, 42.0, 1000.0] {
                let g = create(width, height, extent, 0.05).unwrap();
                assert_eq!(g.extent, extent.min(width / 2.0).min(height / 2.0));
                assert!(g.error_bound <= 0.05);
                assert_convex(&g);
                for corner in &g.corners {
                    close(corner.extent, g.extent, 1e-10);
                }
            }
        }
        let g = create(10.123456789, 20.987654321, 0.0, 0.05).unwrap();
        let rounding = (g.width - fmt(g.width).parse::<f64>().unwrap())
            .hypot(g.height - fmt(g.height).parse::<f64>().unwrap());
        assert!(rounding > 0.0 && rounding <= g.error_bound);
    }

    #[test]
    fn adaptive_error_budgets_bound_independently_evaluated_curves() {
        for [width, height, extent] in [
            [600.0, 400.0, 150.0],
            [536.0, 336.0, 118.0],
            [100.0, 80.0, 27.0],
            [72.0, 56.0, 56.0 * PHOTO_EXTENT],
        ] {
            let g = create(width, height, extent, 0.02).unwrap();
            assert!(g.error_bound <= 0.02);
            assert!(g.corners.iter().all(|c| c.points.len() < 500));
            check_error(&g);
            assert_convex(&g);
        }
    }

    #[test]
    fn all_four_corners_preserve_identical_physical_coordinates() {
        for [width, height, extent] in [
            [240.0, 120.0, 27.0],
            [120.0, 240.0, 27.0],
            [127.0, 127.0, 127.0 * PHOTO_EXTENT],
            [274.0, 144.0, 144.0 * PHOTO_EXTENT],
            [48.0, 44.0, 27.0],
        ] {
            let g = create(width, height, extent, 0.02).unwrap();
            assert!(g.canonical);
            assert_eq!(g.extent, extent.min(width / 2.0).min(height / 2.0));
            for corner in &g.corners {
                assert_eq!(corner.extent, g.extent);
            }
            for i in 0..g.corners[0].points.len() {
                let first = local(g.corners[0].points[i], &g.corners[0], &g, 0);
                for index in 1..4 {
                    let q = local(g.corners[index].points[i], &g.corners[index], &g, index);
                    close(q.x, first.x, 1e-10);
                    close(q.y, first.y, 1e-10);
                }
            }
            check_error(&g);
            assert_convex(&g);
        }
    }

    #[test]
    fn parallel_offsets_resample_analytic_curves_and_reject_singularities() {
        let g = create(200.0, 120.0, 42.0, 0.05).unwrap();
        assert!(offset_path(&g, 3.0).unwrap().starts_with("M42 -3L158 -3"));
        assert_eq!(offset_path(&g, 0.0).unwrap(), g.path);
        assert!(offset_path(&g, -1.0).unwrap().starts_with("M42 1L158 1"));
        let square = create(200.0, 120.0, 0.0, 0.05).unwrap();
        assert!(
            offset_path(&square, -1.0)
                .unwrap()
                .starts_with("M1 1L199 1")
        );
        assert!(offset_path(&square, -60.0).is_err());
        assert!(offset_path(&g, -g.radius).is_err());
        // Check rounded emitted segments against independently evaluated offsets.
        let svg = offset_path(&g, 12.0).unwrap();
        let vertices: Vec<Vector> = svg
            .trim_end_matches('Z')
            .split(['M', 'L'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (x, y) = part.split_once(' ').unwrap();
                Vector {
                    x: x.parse().unwrap(),
                    y: y.parse().unwrap(),
                }
            })
            .collect();
        let distance = |q: Vector, a: Vector, b: Vector| {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let square = dx * dx + dy * dy;
            let t = if square == 0.0 {
                0.0
            } else {
                ((q.x - a.x) * dx + (q.y - a.y) * dy) / square
            }
            .clamp(0.0, 1.0);
            (q.x - a.x - t * dx).hypot(q.y - a.y - t * dy)
        };
        let mut observed: f64 = 0.0;
        for i in 0..=1000 {
            let mut q = reference(
                f64::from(i) / 1000.0,
                Source {
                    offset: -12.0,
                    ..g.corners[0].source
                },
            );
            q.x += 200.0 - 42.0;
            q.y -= 12.0;
            let closest = vertices
                .windows(2)
                .map(|p| distance(q, p[0], p[1]))
                .fold(f64::INFINITY, f64::min);
            observed = observed.max(closest);
        }
        assert!(observed <= 0.05, "offset error {observed}");
    }

    #[test]
    fn bounded_caches_reuse_geometry_and_discard_old_entries() {
        GEOMETRIES.with(|cache| cache.borrow_mut().0.clear());
        let g = create(200.0, 120.0, 42.0, 0.05).unwrap();
        assert!(Rc::ptr_eq(&g, &create(200.0, 120.0, 42.0, 0.05).unwrap()));
        for i in 0..110 {
            create(300.0 + f64::from(i), 150.0, 27.0, 0.05).unwrap();
        }
        assert!(!Rc::ptr_eq(&g, &create(200.0, 120.0, 42.0, 0.05).unwrap()));
        GEOMETRIES.with(|cache| assert_eq!(cache.borrow().0.len(), 96));
        for offset in 1..=20 {
            offset_path(&g, f64::from(offset)).unwrap();
        }
        assert_eq!(g.offset_paths.borrow().0.len(), 16);
    }

    #[test]
    fn invalid_coordinates_budgets_parameters_and_offsets_are_rejected() {
        for input in [
            [0.0, 120.0, 42.0, 0.05],
            [200.0, 120.0, -1.0, 0.05],
            [200.0, 120.0, 42.0, 0.0],
            [200.0, f64::INFINITY, 42.0, 0.05],
            [200.0, 120.0, 42.0, 1e-9],
            [1e16, 120.0, 0.0, 0.05],
        ] {
            assert!(create(input[0], input[1], input[2], input[3]).is_err());
        }
        for t in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            assert!(profile(t).is_err());
        }
        let g = create(200.0, 120.0, 42.0, 0.05).unwrap();
        for offset in [f64::NAN, f64::INFINITY, -f64::INFINITY] {
            assert!(offset_path(&g, offset).is_err());
        }
    }

    #[test]
    fn svg_decimal_format_preserves_javascript_rounding_and_negative_zero() {
        for (input, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (0.0078125, "0.007813"),
            (-0.0078125, "-0.007813"),
            (1.0000015, "1.000001"),
            (0.00000049, "0"),
            (-0.00000049, "0"),
            (12.5, "12.5"),
            (1_000_000_000_000_000_128.0, "1000000000000000100"),
            (1e21, "1e+21"),
            (1e-300, "0"),
            (f64::INFINITY, "Infinity"),
            (-f64::INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(fmt(input), expected, "{input}");
        }
    }

    #[test]
    fn serialized_public_shapes_match_browser_contracts() {
        let geometry = create(200.0, 120.0, 42.0, 0.05).unwrap();
        let json = serde_json::to_value(geometry.as_ref()).unwrap();
        assert!(json.get("errorBound").is_some());
        assert!(json.get("tolerance").is_none());
        assert!(json.get("offset_paths").is_none());
        assert!(json["points"][0].get("parameter").is_none());
        assert!(json["corners"][0]["points"][0].get("parameter").is_some());
        let profile = serde_json::to_value(profile(0.5).unwrap()).unwrap();
        assert!(profile.get("curvatureDerivative").is_some());
        assert!(profile.get("curvatureSecondDerivative").is_some());
        let bounds = serde_json::to_value(bounds()).unwrap();
        assert_eq!(bounds["intervals"], 512);
        assert!(bounds.get("minimumRadius").is_some());
    }
}
