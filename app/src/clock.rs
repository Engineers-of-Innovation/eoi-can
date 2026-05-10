use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, LsConfig, LseConfig, LseDrive, LseMode, Pll, PllMul,
    PllPreDiv, PllRDiv, PllSource, RtcClockSource, Sysclk,
};
use embassy_stm32::time::Hertz;

pub fn clock_config() -> embassy_stm32::Config {
    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(16_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,  // 16 MHz HSE
        prediv: PllPreDiv::DIV1, // /1 → 16 MHz
        mul: PllMul::MUL10,      // ×10 → 160 MHz VCO
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2), // /2 → 80 MHz SYSCLK
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    config.rcc.ls = LsConfig {
        rtc: RtcClockSource::LSE,
        lsi: false,
        lse: Some(LseConfig {
            frequency: Hertz(32_768),
            mode: LseMode::Oscillator(LseDrive::MediumHigh),
        }),
    };
    config.rcc.mux.adcsel = embassy_stm32::rcc::mux::Adcsel::SYS;
    config
}
