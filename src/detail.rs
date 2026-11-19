use cascii_core_view::MultiSizeManifest;
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Curve { Linear, Exponential, Logarithmic }

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Direction { Up, Down }

#[derive(Clone, Copy)]
pub enum Detail {
    Auto,
    Authored,
    Width(u32),
    Transition(Curve, Direction),
}

impl Detail {
    pub fn columns(self, manifest: &MultiSizeManifest, index: usize, width: u16, height: u16) -> u32 {
        let first = manifest.variants.first().unwrap().columns;
        match self {
            Self::Auto => manifest.fitting_columns(index, width.into(), height.into()).unwrap_or(first),
            Self::Authored => manifest.column_map.as_ref().map_or(manifest.default_columns, |map| map[index]),
            Self::Width(width) => manifest.nearest_columns(f64::from(width)).unwrap(),
            Self::Transition(curve, direction) => {
                let t = index as f64 / f64::from((manifest.frame_count - 1).max(1));
                let progress = match curve {
                    Curve::Linear => t,
                    Curve::Exponential => (4.0 * t).exp_m1() / 4.0_f64.exp_m1(),
                    Curve::Logarithmic => (4.0_f64.exp_m1() * t).ln_1p() / 4.0,
                };
                let progress = match direction {Direction::Up => progress, Direction::Down => 1.0 - progress};
                let last = manifest.variants.last().unwrap().columns;
                manifest.nearest_columns(f64::from(first) + f64::from(last - first) * progress).unwrap()
            }
        }
    }

    pub fn step(&mut self, manifest: &MultiSizeManifest, current: u32, up: bool) {
        let variants = &manifest.variants;
        let next = if up {
            variants.iter().find(|v| v.columns > current)
        } else {
            variants.iter().rev().find(|v| v.columns < current)
        };
        *self = Self::Width(next.map_or(current, |v| v.columns));
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Authored => "map",
            Self::Width(_) => "manual",
            Self::Transition(Curve::Linear, _) => "linear",
            Self::Transition(Curve::Exponential, _) => "exp",
            Self::Transition(Curve::Logarithmic, _) => "log",
        }
    }
}
