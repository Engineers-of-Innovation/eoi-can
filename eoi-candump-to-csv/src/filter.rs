use std::collections::BTreeSet;

use anyhow::{anyhow, Context};
use eoi_can_decoder::EoiCanData;

pub const MPPT_INSTANCES: u8 = 8;
pub const GAN_MPPT_INSTANCES: u8 = 16;

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub battery: bool,
    pub vesc: bool,
    pub throttle: bool,
    pub gnss: bool,
    pub rudder: bool,
    pub height: bool,
    pub temperature: bool,
    pub mppt: Option<BTreeSet<u8>>,
    pub gan_mppt: Option<BTreeSet<u8>>,
}

impl Filter {
    pub fn parse(selectors: &[String]) -> anyhow::Result<Self> {
        let mut filter = Filter::default();
        for raw in selectors.iter().flat_map(|s| s.split(',')) {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            filter
                .apply_token(token)
                .with_context(|| format!("invalid --devices value: {token:?}. {}", help_text()))?;
        }
        if !filter.any_selected() {
            return Err(anyhow!("no devices selected. {}", help_text()));
        }
        Ok(filter)
    }

    fn apply_token(&mut self, token: &str) -> anyhow::Result<()> {
        let (family, instance) = match token.split_once(':') {
            Some((fam, idx)) => (fam, Some(idx)),
            None => (token, None),
        };

        match (family, instance) {
            ("all", None) => {
                self.battery = true;
                self.vesc = true;
                self.throttle = true;
                self.gnss = true;
                self.rudder = true;
                self.height = true;
                self.temperature = true;
                self.mppt = Some(BTreeSet::new());
                self.gan_mppt = Some(BTreeSet::new());
            }
            ("battery", None) => self.battery = true,
            ("vesc", None) => self.vesc = true,
            ("throttle", None) => self.throttle = true,
            ("gnss", None) => self.gnss = true,
            ("rudder", None) => self.rudder = true,
            ("height", None) => self.height = true,
            ("temperature", None) => self.temperature = true,
            ("mppt", None) => self.mppt = Some(BTreeSet::new()),
            ("mppt", Some(idx)) => {
                let n: u8 = idx
                    .parse()
                    .with_context(|| format!("not a number: {idx:?}"))?;
                if n >= MPPT_INSTANCES {
                    return Err(anyhow!(
                        "mppt instance {n} out of range 0..{}",
                        MPPT_INSTANCES
                    ));
                }
                self.mppt.get_or_insert_with(BTreeSet::new).insert(n);
            }
            ("gan-mppt", None) => self.gan_mppt = Some(BTreeSet::new()),
            ("gan-mppt", Some(idx)) => {
                let n: u8 = idx
                    .parse()
                    .with_context(|| format!("not a number: {idx:?}"))?;
                if n >= GAN_MPPT_INSTANCES {
                    return Err(anyhow!(
                        "gan-mppt instance {n} out of range 0..{}",
                        GAN_MPPT_INSTANCES
                    ));
                }
                self.gan_mppt.get_or_insert_with(BTreeSet::new).insert(n);
            }
            _ => return Err(anyhow!("unknown selector")),
        }
        Ok(())
    }

    fn any_selected(&self) -> bool {
        self.battery
            || self.vesc
            || self.throttle
            || self.gnss
            || self.rudder
            || self.height
            || self.temperature
            || self.mppt.is_some()
            || self.gan_mppt.is_some()
    }

    pub fn accepts(&self, data: &EoiCanData) -> bool {
        match data {
            EoiCanData::EoiBattery(_) => self.battery,
            EoiCanData::Vesc(_) => self.vesc,
            EoiCanData::Throttle(_) => self.throttle,
            EoiCanData::Gnss(_) => self.gnss,
            EoiCanData::RudderController(_) => self.rudder,
            EoiCanData::HeightSensors(_) => self.height,
            EoiCanData::Temperature(_) => self.temperature,
            EoiCanData::Mppt(m) => match &self.mppt {
                None => false,
                Some(set) if set.is_empty() => true,
                Some(set) => set.contains(&m.node_id()),
            },
            EoiCanData::GanMppt(m) => match &self.gan_mppt {
                None => false,
                Some(set) if set.is_empty() => true,
                Some(set) => set.contains(&m.node_id()),
            },
        }
    }

    pub fn mppt_instances(&self) -> Vec<u8> {
        match &self.mppt {
            None => vec![],
            Some(set) if set.is_empty() => (0..MPPT_INSTANCES).collect(),
            Some(set) => set.iter().copied().collect(),
        }
    }

    pub fn gan_mppt_instances(&self) -> Vec<u8> {
        match &self.gan_mppt {
            None => vec![],
            Some(set) if set.is_empty() => (0..GAN_MPPT_INSTANCES).collect(),
            Some(set) => set.iter().copied().collect(),
        }
    }
}

fn help_text() -> &'static str {
    "valid selectors: all, battery, vesc, throttle, mppt, mppt:N (0..7), gan-mppt, gan-mppt:N (0..15), gnss, rudder, height, temperature"
}

#[cfg(test)]
mod tests {
    use super::*;
    use eoi_can_decoder::{
        BatteryUptime, EoiBattery, GanMpptData, GanMpptPacket, GanMpptStatus, GanPhaseFault,
        GanPhaseMode, MpptData, MpptInfo, MpptPower,
    };

    fn parse(args: &[&str]) -> anyhow::Result<Filter> {
        Filter::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn accepts_family() {
        let f = parse(&["battery"]).unwrap();
        assert!(f.battery);
        assert!(!f.gnss);
        assert!(f.accepts(&EoiCanData::EoiBattery(EoiBattery::BatteryUptime(
            BatteryUptime { uptime_ms: 0 }
        ))));
    }

    #[test]
    fn comma_and_repeat_combine() {
        let f = parse(&["battery,gnss", "rudder"]).unwrap();
        assert!(f.battery && f.gnss && f.rudder);
    }

    #[test]
    fn mppt_specific_instance() {
        let f = parse(&["mppt:3"]).unwrap();
        let frame = EoiCanData::Mppt(
            MpptData::from_node_id(
                3,
                MpptInfo::Power(MpptPower {
                    voltage_out: 0.0,
                    current_out: 0.0,
                }),
            )
            .unwrap(),
        );
        assert!(f.accepts(&frame));

        let other = EoiCanData::Mppt(
            MpptData::from_node_id(
                2,
                MpptInfo::Power(MpptPower {
                    voltage_out: 0.0,
                    current_out: 0.0,
                }),
            )
            .unwrap(),
        );
        assert!(!f.accepts(&other));
    }

    #[test]
    fn mppt_all_instances() {
        let f = parse(&["mppt"]).unwrap();
        for i in 0..MPPT_INSTANCES {
            let frame = EoiCanData::Mppt(
                MpptData::from_node_id(
                    i,
                    MpptInfo::Power(MpptPower {
                        voltage_out: 0.0,
                        current_out: 0.0,
                    }),
                )
                .unwrap(),
            );
            assert!(f.accepts(&frame), "expected mppt:{i} to be accepted");
        }
        assert_eq!(f.mppt_instances(), (0..MPPT_INSTANCES).collect::<Vec<_>>());
    }

    #[test]
    fn gan_mppt_specific_instance() {
        let f = parse(&["gan-mppt:5"]).unwrap();
        let frame = EoiCanData::GanMppt(
            GanMpptData::from_node_id(
                5,
                GanMpptPacket::Status(GanMpptStatus {
                    mode: GanPhaseMode::None,
                    fault: GanPhaseFault::Ok,
                    enabled: false,
                    board_temp: 0,
                    heat_sink_temp: 0,
                }),
            )
            .unwrap(),
        );
        assert!(f.accepts(&frame));
    }

    #[test]
    fn all_selects_everything() {
        let f = parse(&["all"]).unwrap();
        assert!(
            f.battery && f.vesc && f.throttle && f.gnss && f.rudder && f.height && f.temperature
        );
        assert_eq!(f.mppt_instances().len(), MPPT_INSTANCES as usize);
        assert_eq!(f.gan_mppt_instances().len(), GAN_MPPT_INSTANCES as usize);
    }

    #[test]
    fn rejects_unknown_token() {
        assert!(parse(&["nope"]).is_err());
        assert!(parse(&["mppt:9"]).is_err());
        assert!(parse(&["gan-mppt:99"]).is_err());
        assert!(parse(&[]).is_err());
    }
}
