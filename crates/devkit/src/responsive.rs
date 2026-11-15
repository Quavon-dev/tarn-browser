#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
}

pub const PRESETS: [Preset; 5] = [
    Preset { name: "iPhone SE", width: 375.0, height: 667.0 },
    Preset { name: "iPhone 15", width: 393.0, height: 852.0 },
    Preset { name: "iPad", width: 820.0, height: 1180.0 },
    Preset { name: "Laptop", width: 1280.0, height: 800.0 },
    Preset { name: "Desktop", width: 1920.0, height: 1080.0 },
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

pub fn fit(preset: &Preset, available_width: f32, available_height: f32) -> Fit {
    let scale = (available_width / preset.width).min(available_height / preset.height).clamp(0.0, 1.0);
    let width = preset.width * scale;
    let height = preset.height * scale;
    Fit { x: ((available_width - width) / 2.0).max(0.0), y: ((available_height - height) / 2.0).max(0.0), width, height, scale }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centres_without_upscaling() {
        let f = fit(&PRESETS[0], 1000.0, 1000.0);
        assert_eq!(f, Fit { x: 312.5, y: 166.5, width: 375.0, height: 667.0, scale: 1.0 });
    }

    #[test]
    fn scales_down_to_fit() {
        let f = fit(&PRESETS[4], 960.0, 1000.0);
        assert_eq!(f.scale, 0.5);
        assert_eq!((f.width, f.height), (960.0, 540.0));
        assert_eq!((f.x, f.y), (0.0, 230.0));
    }

    #[test]
    fn handles_empty_bounds() {
        let f = fit(&PRESETS[2], 0.0, -5.0);
        assert_eq!((f.scale, f.width, f.height), (0.0, 0.0, 0.0));
    }
}
