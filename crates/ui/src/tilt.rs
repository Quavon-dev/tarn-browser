//! A flat card seen in perspective: it leans away under the pointer, a layer on it rises off its
//! face and it turns over. GPUI cannot transform an element, so the card is projected pixel by pixel.

const TILT: f32 = 0.2;
/// The eye's distance from the card, in pixels.
const EYE: f32 = 1500.;

/// Premultiplied RGBA pixels, row by row.
pub struct Texture<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

pub struct Pose {
    /// Where the pointer is over the card, each axis from -1 to 1.
    pub tilt: (f32, f32),
    /// How far the raised layer has risen, 0 to 1.
    pub glow: f32,
    /// 0 shows the front, 1 the back.
    pub flip: f32,
    /// 0 to 1; pushes the card in.
    pub press: f32,
}

/// The card as straight-alpha BGRA, `margin` pixels larger than `front` on every side. `back` is
/// as large as `front`. `raised` floats above the front, clipped to it: a texture, its left and
/// top on the card and its height above it when `glow` is 1.
pub fn frame(front: &Texture, back: &Texture, raised: (&Texture, f32, f32, f32), margin: u32, pose: &Pose) -> Vec<u8> {
    let (ow, oh) = (front.width + 2 * margin, front.height + 2 * margin);
    let (half_w, half_h) = (front.width as f32 / 2., front.height as f32 / 2.);
    let (sin_y, cos_y) = (pose.tilt.0 * TILT + pose.flip * std::f32::consts::PI).sin_cos();
    let (sin_x, cos_x) = (pose.tilt.1 * TILT).sin_cos();
    let size = 1. - 0.035 * pose.press;
    let ex = [cos_y * size, -sin_y * sin_x * size, sin_y * cos_x * size];
    let ey = [0., cos_x * size, sin_x * size];
    let away = [-sin_y, -cos_y * sin_x, cos_y * cos_x];
    let facing = away[2] > 0.;
    // The card turns about its nearest edge, so no part of it comes closer than it is at rest.
    let depth = sin_y.abs() * half_w + sin_x.abs() * half_h;
    let light = (1. + 0.45 * sin_x + 0.3 * (pose.tilt.0 * TILT).sin()) * (0.5 + 0.5 * cos_y.abs());
    let mut out = vec![0f32; (ow * oh * 4) as usize];
    let mut layer = |tex: &Texture, left: f32, top: f32, lift: f32, clip: bool| {
        let o = [-away[0] * lift, -away[1] * lift, depth - away[2] * lift];
        let (tw, th) = (tex.width as i32, tex.height as i32);
        let corner = |x: f32, y: f32| {
            let p = [o[0] + x * ex[0] + y * ey[0], o[1] + x * ex[1] + y * ey[1], o[2] + x * ex[2] + y * ey[2]];
            (p[0] * EYE / (EYE + p[2]), p[1] * EYE / (EYE + p[2]))
        };
        let corners = [corner(left, top), corner(left + tw as f32, top), corner(left, top + th as f32), corner(left + tw as f32, top + th as f32)];
        let span = |pick: fn(&(f32, f32)) -> f32, centre: f32, limit: u32| {
            let (low, high) = corners.iter().map(pick).fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v), h.max(v)));
            ((low + centre - 2.).max(0.) as u32)..((high + centre + 2.).max(0.) as u32).min(limit)
        };
        let texel = |x: i32, y: i32| -> [f32; 4] {
            if x < 0 || y < 0 || x >= tw || y >= th {
                return [0.; 4];
            }
            let at = ((y * tw + x) * 4) as usize;
            [tex.rgba[at] as f32, tex.rgba[at + 1] as f32, tex.rgba[at + 2] as f32, tex.rgba[at + 3] as f32]
        };
        let columns = span(|c| c.0, ow as f32 / 2., ow);
        for py in span(|c| c.1, oh as f32 / 2., oh) {
            let v = py as f32 + 0.5 - oh as f32 / 2.;
            let (a21, a22, b2) = (EYE * ex[1] - v * ex[2], EYE * ey[1] - v * ey[2], v * (EYE + o[2]) - EYE * o[1]);
            for px in columns.clone() {
                let u = px as f32 + 0.5 - ow as f32 / 2.;
                let (a11, a12, b1) = (EYE * ex[0] - u * ex[2], EYE * ey[0] - u * ey[2], u * (EYE + o[2]) - EYE * o[0]);
                let det = a11 * a22 - a12 * a21;
                if det.abs() < 1e-3 {
                    continue;
                }
                let (mut x, y) = ((b1 * a22 - a12 * b2) / det, (a11 * b2 - b1 * a21) / det);
                if !facing {
                    x = -x;
                }
                let (tx, ty) = (x - left - 0.5, y - top - 0.5);
                let (x0, y0) = (tx.floor(), ty.floor());
                let (fx, fy) = (tx - x0, ty - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                if x0 < -1 || y0 < -1 || x0 >= tw || y0 >= th {
                    continue;
                }
                let (c00, c10, c01, c11) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
                let mix = |i: usize| ((c00[i] * (1. - fx) + c10[i] * fx) * (1. - fy) + (c01[i] * (1. - fx) + c11[i] * fx) * fy) / 255.;
                let at = ((py * ow + px) * 4) as usize;
                let keep = if clip { out[at + 3] } else { 1. };
                let alpha = mix(3) * keep;
                if alpha <= 0. {
                    continue;
                }
                for i in 0..3 {
                    out[at + i] = (mix(i) * keep * light).min(alpha) + out[at + i] * (1. - alpha);
                }
                out[at + 3] = alpha + out[at + 3] * (1. - alpha);
            }
        }
    };
    layer(if facing { front } else { back }, -half_w, -half_h, 0., false);
    if facing {
        layer(raised.0, raised.1 - half_w, raised.2 - half_h, raised.3 * pose.glow, true);
    }
    out.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|c| {
            let a = c[3].max(1e-4);
            [c[2] / a, c[1] / a, c[0] / a, c[3]].map(|v| (v * 255.).round().clamp(0., 255.) as u8)
        })
        .collect()
}
