use std::sync::mpsc::Receiver;

use anyhow::Result;
use display_interface_spi::SPIInterfaceNoCS;
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoFont, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::Text,
};
use esp_idf_hal::{delay, gpio, prelude::*, spi};
use log::info;

#[derive(Debug, Clone, PartialEq)]
pub enum DrawPos {
    Button(u8),
    Pos(Point),
    Box(Rectangle),
}

impl DrawPos {
    pub fn upper_left(&self) -> Point {
        match self {
            DrawPos::Button(b) => Point::new(20 + 98 * (*b as i32), 220),
            DrawPos::Pos(p) => p.clone(),
            DrawPos::Box(r) => r.top_left.clone(),
        }
    }

    pub fn compute_bounding_box(&self, d: Option<&Rectangle>) -> Rectangle {
        let d: Rectangle = match d {
            Some(r) => r.clone(),
            None => match self {
                DrawPos::Box(b) => b.clone(),
                _ => Rectangle {
                    top_left: Point { x: 0, y: 0 },
                    size: Size {
                        width: 0,
                        height: 0,
                    },
                },
            },
        };

        match self {
            DrawPos::Button(b) => Rectangle {
                top_left: Point::new(20 + 94 * (*b as i32), 200),
                size: Size::new(92, 40),
            },
            DrawPos::Pos(p) => Rectangle {
                top_left: Point {
                    x: p.x,
                    y: p.y - d.size.height as i32 + 1,
                },
                size: Size::new(320, d.size.height + 3),
            },
            DrawPos::Box(r) => Rectangle {
                top_left: Point {
                    x: r.top_left.x,
                    y: r.top_left.y - d.size.height as i32 + 3,
                },
                size: r.size.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    Clear {
        color: Rgb565,
        pos: DrawPos,
    },
    Erase {
        color: Rgb565,
    },
    Text {
        pos: DrawPos,
        text: String,
        text_color: Rgb565,
        font: Option<MonoFont<'static>>,
        background: Option<Rgb565>,
    },
}

pub fn draw_loop(
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
            DrawCmd::Clear { color, pos } => {
                let bb = pos.compute_bounding_box(None);

                display
                    .fill_solid(&bb, color)
                    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;
            }
            DrawCmd::Text {
                pos,
                text,
                text_color,
                font,
                background,
            } => {
                let upper_left = pos.upper_left();

                let the_font: &MonoFont<'static> = match &font {
                    Some(v) => v,
                    None => &FONT_10X20,
                };

                let t = Text::new(&text, upper_left, MonoTextStyle::new(the_font, text_color));

                let bb = pos.compute_bounding_box(Some(&t.bounding_box()));
                match background {
                    Some(bc) => display
                        .fill_solid(&bb, bc)
                        .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?,
                    None => (),
                };

                t.draw(&mut display)
                    .map_err(|e| anyhow::anyhow!("Display error: {:?}", e))?;
            }
        };
    }
}
