//! Wing calibrations and boat geometry, from `boat.toml`.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct BoatConfig {
    pub front_wings: FrontWings,
    pub rear_wing: RearWing,
    pub height: Height,
    pub hull: Hull,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FrontWings {
    pub fl: FrontWing,
    pub fr: FrontWing,
    pub stop_tol_us: f32,
}

/// One front wing's servo calibration: AoA is linear in PWM between the stops.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontWing {
    pub us_min: f32,
    pub us_max: f32,
    pub deg_at_min: f32,
    pub deg_at_max: f32,
}

impl FrontWing {
    pub fn aoa(&self, us: f32) -> f32 {
        self.deg_at_min
            + (us - self.us_min) * (self.deg_at_max - self.deg_at_min) / (self.us_max - self.us_min)
    }

    pub fn at_stop(&self, us: f32, tol_us: f32) -> bool {
        (us - self.us_min).abs() <= tol_us || (us - self.us_max).abs() <= tol_us
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RearWing {
    pub zero_sp: f32,
    pub deg_per_unit: f32,
    pub chord_below_mount_deg: f32,
}

impl RearWing {
    /// AoA of the rear foil at a stepper position (or setpoint), in setpoint units.
    ///
    /// Higher setpoint = higher AoA: `zero_sp` (the far stop) is mount 0 deg and the
    /// node's 2000 home stop is the high-AoA end. The rear wing was mounted backwards
    /// until 2026-10-24; this matches hydrofoils.lua c09ae83 after the re-flip.
    pub fn aoa(&self, position: f32) -> f32 {
        (position - self.zero_sp) * self.deg_per_unit - self.chord_below_mount_deg
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Height {
    pub breach_m: f32,
    pub foil_depth_at_h0_m: f32,
    pub rest_m: f32,
}

impl Height {
    /// How far the foils sit below the keel, which is the drawn strut length.
    pub fn strut_below_keel_m(&self) -> f32 {
        self.foil_depth_at_h0_m - self.rest_m
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hull {
    pub length_m: f32,
    pub front_foil_x_m: f32,
    pub rear_foil_x_m: f32,
    pub depth_m: f32,
    pub beam_m: f32,
    pub half_track_m: f32,
    pub foil_chord_m: f32,
    pub foil_span_m: f32,
}

const BUILT_IN: &str = include_str!("../boat.toml");

impl Default for BoatConfig {
    fn default() -> Self {
        toml::from_str(BUILT_IN).expect("the built-in boat.toml parses")
    }
}

impl BoatConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The numbers the Python overlay (geometry.py) gives for the same calibration.
    #[test]
    fn front_wings_match_the_overlay() {
        let cfg = BoatConfig::default();
        let (fl, fr) = (&cfg.front_wings.fl, &cfg.front_wings.fr);
        assert_eq!(fl.aoa(1435.0), -7.5);
        assert_eq!(fl.aoa(1730.0), 9.5);
        // FR is reversed: its low PWM is its high AoA.
        assert_eq!(fr.aoa(1300.0), 9.5);
        assert_eq!(fr.aoa(1540.0), -7.5);
        assert!((fl.aoa(1582.5) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn stops_have_a_tolerance() {
        let cfg = BoatConfig::default();
        let fl = &cfg.front_wings.fl;
        assert!(fl.at_stop(1437.0, 2.0));
        assert!(fl.at_stop(1728.0, 2.0));
        assert!(!fl.at_stop(1438.0, 2.0));
        assert!(!fl.at_stop(1600.0, 2.0));
    }

    #[test]
    fn rear_wing_homes_at_the_high_aoa_stop() {
        let rear = BoatConfig::default().rear_wing;
        assert_eq!(rear.aoa(1000.0), -3.0);
        assert!((rear.aoa(2000.0) - 9.0).abs() < 1e-4);
    }

    #[test]
    fn strut_length() {
        let h = BoatConfig::default().height;
        assert!((h.strut_below_keel_m() - 0.546).abs() < 1e-6);
    }
}
