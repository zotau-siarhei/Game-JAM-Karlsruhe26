// Пламя на карточке: два слоя шума бегут вверх и рвут маску «капли», цвет — по рампе
// от тёмно-красного края к белому ядру ярче единицы (светится через свечение камеры).
// Сила огня `flame.x` от 0 до 1 съедает маску: слабый огонь ниже и рванее.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::globals;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> flame: vec4<f32>;

// Случайное число ячейки шума: целые умножения и сдвиги. Прежний `fract(sin(…))` рос со временем игры,
// а точность `sin` больших чисел на телефонных видеокартах не гарантирована.
fn hash(p: vec2<f32>) -> f32 {
    let cell = vec2<u32>(vec2<i32>(p));
    var h = (cell.x * 0x9E3779B1u) ^ (cell.y * 0x85EBCA77u);
    h = (h ^ (h >> 15u)) * 0x2C1B3C6Du;
    h = (h ^ (h >> 12u)) * 0x297A2D39u;
    h = h ^ (h >> 15u);
    return f32(h >> 8u) / 16777216.0;
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.55;
    var q = p;
    for (var i = 0; i < 3; i++) {
        sum += value_noise(q) * amp;
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        amp *= 0.5;
    }
    return sum;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let strength = clamp(flame.x, 0.0, 1.0);
    // Низ карточки — v = 1, верх — v = 0.
    let x = (in.uv.x - 0.5) * 2.0;
    let y = 1.0 - in.uv.y;
    // Фаза из мировой позиции: соседние огни не горят в такт.
    let phase = in.world_position.x * 0.37 + in.world_position.z * 0.61;
    let t = globals.time;
    let n1 = fbm(vec2<f32>(x * 1.6 + phase, y * 2.2 - t * 1.7));
    let n2 = fbm(vec2<f32>(x * 3.4 - phase * 1.3, y * 4.5 - t * 2.9));
    let n = n1 * 0.65 + n2 * 0.35;
    // Капля: широкий круглый низ, узкий верх, язык качается шумом тем сильнее, чем выше.
    let sway = (n1 - 0.5) * 0.6 * y;
    let half = 0.9 * pow(max(1.0 - y, 0.0), 0.55) + 0.06;
    let r = abs(x + sway) / half;
    let body = clamp(1.0 - r * r, 0.0, 1.0);
    let base = smoothstep(0.0, 0.1, y);
    // Эрозия: порог растёт с высотой, шум выедает края и рвёт верх на языки; слабый огонь ниже.
    let threshold = (0.22 + 0.62 * y) * (1.3 - 0.45 * strength);
    let heat = body * base * (0.35 + 1.3 * n) - threshold;
    if heat <= 0.0 {
        discard;
    }
    let edge = vec3<f32>(0.7, 0.06, 0.0);
    let orange = vec3<f32>(2.2, 0.42, 0.02);
    let yellow = vec3<f32>(2.8, 1.25, 0.1);
    let core = vec3<f32>(3.2, 2.3, 0.8);
    var color = mix(edge, orange, smoothstep(0.0, 0.12, heat));
    color = mix(color, yellow, smoothstep(0.18, 0.4, heat));
    color = mix(color, core, smoothstep(0.45, 0.75, heat));
    let alpha = smoothstep(0.0, 0.08, heat);
    return vec4<f32>(color, alpha);
}
