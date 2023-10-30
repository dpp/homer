use std::time::Duration;

use anyhow::Result;
use crossbeam::channel::Sender;
use esp_idf_hal::{
    adc::{attenuation, config::Config, AdcChannelDriver, AdcDriver, ADC1},
    gpio::Gpio1,
};

fn reading_to_button(reading: u16) -> Option<u8> {
    if reading > 700 && reading < 1000 {
        Some(2)
    } else if reading > 1800 && reading < 2200 {
        Some(1)
    } else if reading > 2300 && reading < 2600 {
        Some(0)
    } else {
        None
    }
}

pub fn button_loop(button_tx: Sender<usize>, gpio1: Gpio1, adc1: ADC1) -> Result<()> {
    let mut adc = AdcDriver::new(adc1, &Config::new().calibration(true))?;
    let mut adc_pin = AdcChannelDriver::<{ attenuation::DB_11 }, Gpio1>::new(gpio1)?;

    // 700-900 button 3
    // 1900-2200 button 2
    // 2300-2500 button 1

    // FIXME - debounce

    let mut cur = reading_to_button(adc.read(&mut adc_pin).unwrap());
    let mut last = [false, false, false];
    loop {
        let now = reading_to_button(adc.read(&mut adc_pin).unwrap());
        let mut this = [false, false, false];
        match now {
            Some(v) => this[v as usize] = true,
            _ => (),
        };

        if last != this {
            for x in 0..3 {
                if this[x] && this[x] != last[x] {
                    button_tx.send(x).unwrap();
                }
            }

            last = this;
        }

        if now != cur {
            cur = now;
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}
