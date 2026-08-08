use defmt::error;
use embassy_stm32::pac::RCC;
use embassy_stm32::rcc::{
    AHBPrescaler, APBPrescaler, Hse, HseMode, LsConfig, Pll, PllMul, PllPreDiv, PllRDiv, PllSource,
    Sysclk,
};
use embassy_stm32::time::Hertz;

/// Bring the HSE up ourselves, with a bound.
///
/// `embassy_stm32::init` waits on HSERDY in a loop with no timeout and no
/// fallback, so a crystal that does not oscillate bricks the board silently --
/// no log line, no fault, nothing to attach a debugger to. Probing it here
/// first turns that into a diagnosable degraded boot.
///
/// Runs before `init`, i.e. on the reset-default MSI 4 MHz clock, so the
/// iteration count is a coarse stand-in for roughly half a second. HSE startup
/// is typically under 2 ms. Leaves HSEON clear on failure so the later `init`
/// (configured for HSI by then) is not left with a half-started oscillator.
fn hse_starts() -> bool {
    RCC.cr().modify(|w| {
        w.set_hsebyp(false);
        w.set_hseon(true);
    });
    for _ in 0..200_000 {
        if RCC.cr().read().hserdy() {
            return true;
        }
        cortex_m::asm::nop();
    }
    RCC.cr().modify(|w| w.set_hseon(false));
    false
}

/// Returns the clock config, and whether the 16 MHz crystal actually started.
///
/// Both paths produce an identical 80 MHz tree: HSI16 is exactly 16 MHz on this
/// part, so the same DIV1/MUL10/DIVR2 chain applies either way and no peripheral
/// divisor downstream has to care.
pub fn clock_config() -> (embassy_stm32::Config, bool) {
    let mut config = embassy_stm32::Config::default();

    let hse_ok = hse_starts();
    if hse_ok {
        config.rcc.hse = Some(Hse {
            freq: Hertz(16_000_000),
            mode: HseMode::Oscillator,
        });
    } else {
        // The board lives and logs, but HSI16 is +/-1% at the factory and worse
        // over temperature, where CAN at 1 Mbit wants better than ~0.5%. This is
        // a bring-up aid, not something to ship on.
        error!("HSE did not start - falling back to HSI16. CAN WILL BE UNRELIABLE.");
        config.rcc.hsi = true;
    }
    config.rcc.pll = Some(Pll {
        source: if hse_ok {
            PllSource::HSE
        } else {
            PllSource::HSI
        },
        prediv: PllPreDiv::DIV1, // /1 -> 16 MHz
        mul: PllMul::MUL10,      // x10 -> 160 MHz VCO
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2), // /2 -> 80 MHz SYSCLK
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.apb2_pre = APBPrescaler::DIV1;
    // No 32.768 kHz crystal is fitted and nothing in this firmware touches the
    // RTC. Enabling the LSE makes embassy spin forever on LSERDY in rcc/bd.rs,
    // and that loop runs *before* the HSE one, so a missing X2 hangs the boot
    // outright. `embassy_time`'s tick-hz-32_768 is unrelated: time-driver-tim4
    // divides TIM4's 80 MHz APB1 clock down to that rate.
    config.rcc.ls = LsConfig::off();

    (config, hse_ok)
}
