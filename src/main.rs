use anyhow::Result;
use display_interface_spi::SPIInterfaceNoCS;
use embedded_graphics::mono_font::{ascii::FONT_10X20, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use embedded_graphics::text::*;

use chrono::{Datelike, Timelike, Utc};
use esp_idf_hal::delay;
use esp_idf_hal::gpio::{self};
use esp_idf_hal::peripheral;
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi::{self};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use esp_idf_svc::sntp::{SyncMode, SyncStatus};
use esp_idf_svc::{sntp, wifi::*};
use esp_idf_sys as _; // If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use log::*;
use mipidsi;

use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use embedded_svc::wifi::{ClientConfiguration, Configuration};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, wifi::EspWifi};

use esp_idf_hal::adc::config::Config;
use esp_idf_hal::adc::*;

fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Hello, world!");

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let pins = peripherals.pins;
    //  let mut nvs = EspDefaultNvsPartition::take()?;

    let (tx, rx) = mpsc::channel::<DrawCmd>();

    std::thread::Builder::new()
        .stack_size(20000)
        .spawn(move || {
            draw_loop(
                rx,
                pins.gpio45,
                pins.gpio4,
                pins.gpio48,
                peripherals.spi2,
                pins.gpio7,
                pins.gpio6,
                pins.gpio5,
            )
            .unwrap();
        })?;

    tx.send(DrawCmd::Erase {
        color: Rgb565::WHITE,
    })?;

    use esp_idf_hal::adc;

    let mut adc = AdcDriver::new(peripherals.adc1, &Config::new().calibration(true))?;
    let mut adc_pin = AdcChannelDriver::<_, adc::Atten11dB<adc::ADC1>>::new(pins.gpio1)?;

    fn reading_to_button(reading: u16) -> Option<u8> {
        if reading > 700 && reading < 1000 {
            Some(3)
        } else if reading > 1800 && reading < 2200 {
            Some(2)
        } else if reading > 2300 && reading < 2600 {
            Some(1)
        } else {
            None
        }
    }

    std::thread::Builder::new()
        .stack_size(3000)
        .spawn(move || {
            // 700-900 button 3
            // 1900-2200 button 2
            // 2300-2500 button 1

            let mut cur = reading_to_button(adc.read(&mut adc_pin).unwrap());
            loop {
                let now = reading_to_button(adc.read(&mut adc_pin).unwrap());
                if now != cur {
                    println!("Level {:?}", now);
                    cur = now;
                }

                std::thread::sleep(Duration::from_millis(50));
            }
        })?;

    let txc = tx.clone();

    std::thread::Builder::new()
        .stack_size(20000)
        .spawn(move || {
            let mut _wifi = wifi(peripherals.modem, sysloop.clone()).unwrap();

            test_https_client().unwrap();
            info!("Done with http!");

            let _sntp = sntp::EspSntp::new_default().unwrap();

            info!("SNTP initialized");

            let mut cnt = 0;
            let mut not_sync = true;
            loop {
                cnt += 1;
                txc.send(DrawCmd::Text {
                    pos: DrawPos::Button1,
                    text: format!("{} {}", cnt, if not_sync {" *"} else {""}),
                    text_color: Rgb565::BLUE,
                    background: Some(Rgb565::WHITE),
                })
                .unwrap();
                std::thread::sleep(Duration::from_secs(7));
                if not_sync {
                    let status: SyncStatus = _sntp.get_sync_status();
                    match status {
                      SyncStatus::Completed => {
                        not_sync = false;
                      },
                      SyncStatus::InProgress => {
                        println!("Sync in progress");
                      },
                      SyncStatus::Reset => {
                        println!("SNTP reset");
                      }
                    }
                }
            }
        })?;

    let mut cnt = 190_235;
    loop {
        tx.send(DrawCmd::Text {
            pos: DrawPos::Button2,
            text: format!("{}", cnt),
            text_color: Rgb565::GREEN,
            background: Some(Rgb565::WHITE),
        })?;

        tx.send(DrawCmd::Text {
            pos: DrawPos::Pos(Point::new(10, 20)),
            text: format!("{}", Utc::now()),
            text_color: RgbColor::BLACK,
            background: Some(RgbColor::WHITE),
        })?;
        cnt += 1;
        std::thread::sleep(Duration::from_secs(1));
    }
    // Ok(())
}

const SSID: &str = env!("SSID");
const PASS: &str = env!("WIFI_PASSWORD");

fn _get_nvs_count() -> Result<u32> {
    let nvs_default_partition: EspNvsPartition<NvsDefault> = EspDefaultNvsPartition::take()?;

    let test_namespace = "test_ns";
    let nvs = EspNvs::new(nvs_default_partition, test_namespace, true)?;

    let tag_u8 = "count";

    let cur = match nvs.get_u32(tag_u8)? {
        Some(v) => v,
        None => 0,
    } + 1;

    nvs.set_u32(tag_u8, cur)?;

    Ok(cur)
}

fn wifi(
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.start()?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        password: PASS.into(),

        ..Default::default()
    }))?;

    wifi.connect()?;

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Wifi DHCP info: {:?}", ip_info);

    //    ping(ip_info.subnet.gateway)?;

    Ok(Box::new(esp_wifi))
}

fn test_https_client() -> anyhow::Result<()> {
    use embedded_svc::http::client::*;
    use embedded_svc::utils::io;
    use esp_idf_svc::http::client::*;

    let url = String::from("https://blog.goodstuff.im");

    info!("About to fetch content from {}", url);

    let mut client = Client::wrap(EspHttpConnection::new(&Configuration {
        crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),

        ..Default::default()
    })?);

    let mut response = client.get(&url)?.submit()?;

    let mut body = [0_u8; 3048];

    let read = io::try_read_full(&mut response, &mut body).map_err(|err| err.0)?;

    info!(
        "Body (truncated to 3K):\n{:?}",
        String::from_utf8_lossy(&body[..read]).into_owned()
    );

    // Complete the response
    while response.read(&mut body)? > 0 {}

    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum DrawPos {
    Button1,
    Button2,
    Button3,
    Pos(Point),
    Box(Rectangle),
}

impl DrawPos {
    pub fn upper_left(&self) -> Point {
        match self {
            DrawPos::Button1 => Point::new(20, 220),
            DrawPos::Button2 => Point::new(114, 220),
            DrawPos::Button3 => Point::new(207, 220),
            DrawPos::Pos(p) => p.clone(),
            DrawPos::Box(r) => r.top_left.clone(),
        }
    }

    pub fn compute_bounding_box(&self, d: &Rectangle) -> Rectangle {
        match self {
            DrawPos::Button1 => Rectangle {
                top_left: Point::new(20, 200),
                size: Size::new(92, 40),
            },
            DrawPos::Button2 => Rectangle {
                top_left: Point::new(114, 200),
                size: Size::new(92, 40),
            },
            DrawPos::Button3 => Rectangle {
                top_left: Point::new(207, 200),
                size: Size::new(92, 40),
            },
            DrawPos::Pos(p) => Rectangle {
                top_left: Point {
                    x: p.x,
                    y: p.y - d.size.height as i32 + 1,
                },
                size: Size::new(320, d.size.height + 1),
            },
            DrawPos::Box(r) => Rectangle {
                top_left: Point {
                    x: r.top_left.x,
                    y: r.top_left.y - d.size.height as i32 + 1,
                },
                size: r.size.clone(),
            },
        }
    }
}

pub enum DrawCmd {
    Erase {
        color: Rgb565,
    },
    Text {
        pos: DrawPos,
        text: String,
        text_color: Rgb565,
        background: Option<Rgb565>,
    },
}

fn draw_loop(
    rx: Receiver<DrawCmd>,
    backlight: gpio::Gpio45,
    dc: gpio::Gpio4,
    rst: gpio::Gpio48,
    spi: spi::SPI2,
    sclk: gpio::Gpio7,
    sdo: gpio::Gpio6,
    cs: gpio::Gpio5,
) -> Result<()> {
    info!("About to initialize the TTGO ST7789 LED driver");

    let mut backlight = gpio::PinDriver::output(backlight)?;
    backlight.set_low()?;

    let di = SPIInterfaceNoCS::new(
        spi::SpiDeviceDriver::new_single(
            spi,
            sclk,
            sdo,
            Option::<gpio::Gpio21>::None,
            Some(cs),
            &spi::SpiDriverConfig::new().dma(spi::Dma::Disabled),
            &spi::SpiConfig::new().baudrate(26.MHz().into()),
        )?,
        gpio::PinDriver::output(dc)?,
    );

    let mut display = mipidsi::Builder::st7789(di)
        .with_display_size(240, 320)
        .with_invert_colors(mipidsi::ColorInversion::Inverted)
        .with_orientation(mipidsi::options::Orientation::LandscapeInverted(true))
        .init(&mut delay::Ets, Some(gpio::PinDriver::output(rst)?))
        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;

    loop {
        let v = rx.recv()?;

        match v {
            DrawCmd::Erase { color } => {
                display
                    .clear(color)
                    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;
            }
            DrawCmd::Text {
                pos,
                text,
                text_color,
                background,
            } => {
                let upper_left = pos.upper_left();

                let t = Text::new(
                    &text,
                    upper_left,
                    MonoTextStyle::new(&FONT_10X20, text_color),
                );

                let bb = pos.compute_bounding_box(&t.bounding_box());
                match background {
                    Some(bc) => {
                        
                        display
                            .fill_solid(&bb, bc)
                            .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?
                    }
                    None => (),
                };

                t.draw(&mut display)
                    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;
            }
        };
    }
}
