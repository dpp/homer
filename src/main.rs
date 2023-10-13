use anyhow::Result;
use chrono::{Local, Timelike};
use crossbeam::select;
use embedded_graphics::{
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::{Point, RgbColor},
};
use esp_idf_hal::prelude::*;
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use esp_idf_sys::{self as _, esp_read_mac, ESP_OK};
use json::JsonValue;
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{atomic::AtomicI32, mpsc::Sender},
};
// If using the `binstart` feature of `esp-idf-sys`, always keep this module imported
use log::*;

use profont::PROFONT_24_POINT;

use crossbeam::channel::bounded;
use homer::{
    buttons::*,
    display::*,
    files::{mount_spiffs, read_file},
    util::*,
    wifi::*,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self},
    Arc,
};
use std::time::Duration;

static HAS_WIFI: AtomicBool = AtomicBool::new(false);
static HAS_TIME: AtomicBool = AtomicBool::new(false);
static LAST_QUAD: AtomicI32 = AtomicI32::new(-1);

fn fetch_config() -> Vec<HAConnect> {
    let mut mac_buffer: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 0];
    let ok = unsafe {
        esp_read_mac(
            mac_buffer.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        )
    };
    let filename: String = if ok == ESP_OK {
        format!(
            "{:02x}_{:02x}_{:02x}",
            mac_buffer[3], mac_buffer[4], mac_buffer[5],
        )
    } else {
        "base".into()
    };

    let conf_string = match read_file(&format!("{}.json", filename))
        .or_else(|_| read_file("base.json"))
        .ok()
    {
        Some(v) => v,
        None => "this_is_bad".into(),
    };
    match serde_json::from_str(&conf_string) {
        Ok(v) => v,
        Err(e) => {
            info!("Failed to parse JSON for {} error {:?}", filename, e);
            vec![HAConnect::Text {
                line: 0,
                text: "Failed to load config!".into(),
                color: 0,
            }]
        }
    }
}

fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // set timezone see https://www.gnu.org/software/libc/manual/html_node/TZ-Variable.html
    std::env::set_var("TZ", env!("HOMER_TZ"));
    unsafe {
        esp_idf_sys::tzset();
    };

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let pins = peripherals.pins;
    let mut _nvs = EspDefaultNvsPartition::take()?;

    mount_spiffs()?;

    info!("Spiffs mounted!");

    let (display_tx, display_rx) = mpsc::channel::<DrawCmd>();

    let (button_tx, button_rx) = bounded::<usize>(5);

    let (ha_tx, ha_rx) = bounded::<Arc<JsonValue>>(60);

    let (socket_tx, socket_rx) = mpsc::channel::<SocketCmd>();

    let main_socket_tx = socket_tx.clone();

    // colors
    // red  0xf800 63488
    // green  0x7e0 2016
    // blue  0x1f 31
    // magenta  0xf81f 63519
    // yellow  0xffe3 65507
    // cyan  0x7ff 2047
    // black 0x0 0
    // white 0xffff 65535

    std::thread::Builder::new()
        .stack_size(10000)
        .spawn(move || {
            draw_loop(
                display_rx,
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

    // clear the screen
    display_tx.send(DrawCmd::Erase {
        color: Rgb565::WHITE,
    })?;

    // start the thread that watches for button presses
    std::thread::Builder::new()
        .stack_size(3000)
        .spawn(move || {
            button_loop(button_tx, pins.gpio1, peripherals.adc1).unwrap();
        })?;

    // start the thread that handles websockets
    std::thread::Builder::new()
        .stack_size(4000)
        .spawn(move || {
            handle_websocket(&HAS_WIFI, socket_tx, socket_rx, ha_tx, HA_AUTH, HA_URL).unwrap();
        })?;

    let display_tx_2 = display_tx.clone();

    // start the thread that deals with wifi
    std::thread::Builder::new()
        .stack_size(5000)
        .spawn(move || {
            // hold the reference so it doesn't get released
            create_wifi(
                SSID,
                PASS,
                &HAS_WIFI,
                &LAST_QUAD,
                display_tx_2,
                peripherals.modem,
                sysloop.clone(),
                &HAS_TIME,
            )
            .unwrap();
        })?;

    // the main event loop
    let mut last_time: String = "".into();
    let mut first_sample = false;
    let mut last_state: HashMap<String, String> = HashMap::new();
    let mut states = HashMap::new();
    let mut ha_config: Vec<HAConnect> = vec![];

    loop {
        // if we haven't sampled, but wifi is up, get the values for the stuff
        // we're watching
        if !first_sample && HAS_WIFI.load(Ordering::Relaxed) {
            // the WIFI is up which means we've got the last quad which means we can load
            // the correct config
            ha_config = fetch_config();
            for connect in &ha_config {
                states.insert(connect.ha_id().clone(), "".to_string());
            }
            for c in &ha_config {
                match get_ha_state(&c.ha_id(), HA_URL, &HA_HEADERS) {
                    Ok(json) => {
                        let val = &json["state"];
                        states.insert(c.ha_id().clone(), val.to_string());
                    }
                    Err(e) => {
                        info!("Failed to get state for {} error {:?}", c.ha_id(), e);
                    }
                }
            }
            first_sample = true;

            // render the layout
            render_states(&ha_config, &states, &mut last_state, &display_tx);
        }

        // if the SNTP server has been connected and we've got time, display it
        if HAS_TIME.load(Ordering::Relaxed) {
            let now = Local::now();
            let this_time = format!("{:>9}:{:0>2}", now.hour(), now.minute());
            if this_time != last_time {
                display_tx.send(DrawCmd::Text {
                    pos: DrawPos::Pos(Point::new(10, 20)),
                    font: Some(PROFONT_24_POINT),
                    text: this_time.clone(),
                    text_color: RgbColor::BLACK,
                    background: Some(RgbColor::WHITE),
                })?;
                last_time = this_time;
            }
        }

        // receive from various channels and perform appropriate actions
        select! {
          // button press
          recv(button_rx) -> msg => {
            let the_button = msg?;
            for c in &ha_config {
              // find the button (there are < 10 items so the cost of looping is low even though it's O(n))
              match c {
                  // find the button
                  HAConnect::Button{button, action_off, action_on, ..} if (*button as usize) == the_button=> {
                    // is it on?
                    let on = c.is_on(&states);
                    // select the command
                    let cmd = if on {action_off} else {action_on};
                    // turn it into a JSON message for Home Assistant
                    let json = cmd.as_json();
                    // send it
                    main_socket_tx.send(SocketCmd::SendJson(json))?;
                  }
                  _ => {}
              }
            }
          },
          // maybe a Home Assistant JSON web socket message
          recv(ha_rx) -> msg => {
            match msg {
              Ok(json) => {
                let json: &JsonValue = json.deref();
                // get the entity_id
                let entity = traverse(json, &["event","data","entity_id"]);
                let mut changed = false;

                // if we've got an 'entity_id' and it's one of the states we care about, update the state table
                // and flag that there's been a change (why?... no need to redraw if there's no change)
                if let Some(s) = &entity {
                  if states.contains_key(s) {
                    if let Some(v) = traverse(json, &["event","data","new_state","state"]) {
                      states.insert(s.clone(), v);
                      changed = true;
                    }
                  }
                }

                // if there's been a change, update the display
                if changed {
                  render_states(&ha_config, &states, &mut last_state, & display_tx);
                }
            },

            Err(_) => {}
          }
        },

        // timeout after a second so we can properly redraw the time even if
        // nothing else has changed
        default(Duration::from_secs(1)) => {}
        };
    }
    // Ok(())
}

// update the display, only rendering states that have changed
fn render_states(
    connect: &[HAConnect],
    states: &HashMap<String, String>,
    last_state: &mut HashMap<String, String>,
    display_tx: &Sender<DrawCmd>,
) {
    for c in connect {
        match c {
            HAConnect::Text { line, text, color } => {
                let cu16: RawU16 = (*color).into();
                
                // don't redisplay
                if Some(text) != last_state.get(text) {
                    last_state.insert(text.clone(), text.clone());
                    display_tx
                        .send(DrawCmd::Text {
                            pos: DrawPos::Pos(Point::new(10, 30 * (*line as i32 + 2))),
                            font: Some(PROFONT_24_POINT),
                            text: text.clone(),
                            text_color: cu16.into(),
                            background: Some(RgbColor::WHITE),
                        })
                        .unwrap();
                }
            }
            HAConnect::Line {
                line,
                ha_id,
                text,
                make_int,
                color,
                ..
            } => {
                if let Some(st) = states.get(ha_id) {
                    let line_str = if *make_int {
                        format!(
                            "{}{}",
                            text,
                            st.parse::<f64>()
                                .ok()
                                .map_or("".to_string(), |f| f.round().to_string())
                        )
                    } else {
                        format!("{}{}", text, st)
                    };

                    if Some(&line_str) != last_state.get(ha_id) {
                        last_state.insert(ha_id.clone(), line_str.clone());

                        let cu16: RawU16 = (*color).into();

                        display_tx
                            .send(DrawCmd::Text {
                                pos: DrawPos::Pos(Point::new(10, 30 * (*line as i32 + 2))),
                                font: Some(PROFONT_24_POINT),
                                text: line_str,
                                text_color: cu16.into(),
                                background: Some(RgbColor::WHITE),
                            })
                            .unwrap();
                    }
                }
            }
            HAConnect::Button {
                button,
                ha_id,
                cmp,
                text_on,
                text_off,
                color,
                ..
            } => {
                let cur = states.get(ha_id);
                let on = cmp == cur;
                let disp = if on { text_on } else { text_off };
                let last = last_state.get(ha_id);
                let cu16: RawU16 = (*color).into();
                if Some(disp) != last {
                    last_state.insert(ha_id.clone(), disp.clone());
                    display_tx
                        .send(DrawCmd::Text {
                            pos: DrawPos::Button(*button),
                            font: None,
                            text: disp.clone(),
                            text_color: cu16.into(),
                            background: Some(RgbColor::WHITE),
                        })
                        .unwrap();
                }
            }
        }
    }
}

const SSID: &str = env!("HOMER_SSID");
const PASS: &str = env!("HOMER_WIFI_PASSWORD");
const HA_AUTH: &str = env!("HOMER_HA_AUTH");
const HA_URL: &str = env!("HOMER_HA_URL");
const HA_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "application/json"),
    ("Authorization", concat!("Bearer ", env!("HOMER_HA_AUTH"))),
];
