use anyhow::{bail, Result};
use crossbeam::channel::Sender as XBSender;

use embedded_graphics::{
    prelude::{Point, RgbColor, Size},
    primitives::Rectangle,
};
use embedded_svc::{
    wifi::{ClientConfiguration, Configuration},
    ws::FrameType,
};
use esp_idf_hal::{modem::Modem, peripheral, io::EspIOError};
use esp_idf_svc::{
    eventloop::{EspEventLoop, EspSystemEventLoop, System},
    sntp::{self, SyncStatus},
    wifi::{BlockingWifi, EspWifi},
    ws::client::{
        EspWebSocketClient, EspWebSocketClientConfig, WebSocketEvent, WebSocketEventType,
    },
};
use json::{object, JsonValue};
use log::*;
use profont::PROFONT_24_POINT;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc::{Receiver, Sender},
        Arc,
    },
    time::Duration,
};

use crate::display::{DrawCmd, DrawPos};

pub enum SocketCmd {
    Reconnect,
    SendString(String),
    SendJson(JsonValue),
}

fn js(s: &str) -> JsonValue {
    JsonValue::String(s.into())
}

pub fn handle_websocket(
    has_wifi: &AtomicBool,
    socket_tx: Sender<SocketCmd>,
    socket_rx: Receiver<SocketCmd>,
    ha_tx: XBSender<Arc<JsonValue>>,
    auth_token: &'static str,
    ha_url: &'static str,
) -> Result<()> {
    // wait until there's a wifi stack
    while !has_wifi.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }

    let socket_to_me = move |info: & Result<WebSocketEvent<'_>, EspIOError>| {
        let auth_okay = js("auth_ok");

        match info {
            Err(e) => {
                info!("Web socket error {:?}", e);
                socket_tx.send(SocketCmd::Reconnect).unwrap();
            }
            Ok(WebSocketEvent {
                event_type: WebSocketEventType::Connected,
                ..
            }) => {
                socket_tx
                    .send(SocketCmd::SendJson(
                        object! {type: "auth", access_token: auth_token},
                    ))
                    .unwrap();
            }
            Ok(WebSocketEvent {
                event_type: WebSocketEventType::Disconnected,
                ..
            }) => {
                socket_tx.send(SocketCmd::Reconnect).unwrap();
            }
            Ok(WebSocketEvent {
                event_type: WebSocketEventType::Text(data),
                ..
            }) => {
                match json::parse(data) {
                    Ok(json) => {
                        if json["type"] == auth_okay {
                            socket_tx
                                .send(SocketCmd::SendJson(object! {
                                 id: 42,
                                type: "subscribe_events"}))
                                .unwrap();
                        } else {
                            ha_tx.send(Arc::new(json)).unwrap();
                        }
                    }
                    Err(_e) => {
                        // info!("Failed to parse JSON {}", e);
                    }
                }
            }

            _ => {}
        }
    };

    let mut socket_client: Option<EspWebSocketClient> = None;
    loop {
        match &socket_client {
            None => {
                info!("Connecting to web socket at {}", ha_url);
                let mut config = EspWebSocketClientConfig::default();
                config.buffer_size = 2048;
                let tmp_socket_client = EspWebSocketClient::new(
                    &format!("ws://{}/api/websocket", ha_url),
                    &config,
                    Duration::from_secs(35),
                    socket_to_me.clone(),
                )
                .ok();
                socket_client = tmp_socket_client;
                if socket_client.is_none() {
                    // if we didn't get a socket, wait...
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
            _ => {}
        }

        if socket_client.is_some() {
            match socket_rx.recv() {
                Err(e) => {
                    info!("Socket error {:?}", e);
                    bail!("Socket Error {:?}", e); // the socket has been closed
                }
                Ok(SocketCmd::Reconnect) => socket_client = None,
                Ok(SocketCmd::SendString(str)) => match &mut socket_client {
                    Some(e) => {
                        match e.send(FrameType::Text(false), str.as_bytes()) {
                            Ok(_) => {}
                            Err(e) => {
                                info!("Socket send error {:?}", e);
                                socket_client = None;
                            }
                        };
                    }
                    None => {}
                },
                Ok(SocketCmd::SendJson(json)) => match &mut socket_client {
                    Some(e) => {
                        match e.send(FrameType::Text(false), json.to_string().as_bytes()) {
                            Ok(_) => {}
                            Err(e) => {
                                info!("Socket send error {:?}", e);
                                socket_client = None;
                            }
                        };
                    }
                    None => {}
                },
            }
        }
    }
}

fn wifi(
    ssid: &'static str,
    password: &'static str,
    has_wifi: &AtomicBool,
    last_quad: &AtomicI32,

    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.start()?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid.into(),
        password: password.into(),

        ..Default::default()
    }))?;

    wifi.connect()?;

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    last_quad.store((ip_info.ip.octets()[3]) as i32, Ordering::Relaxed);

    has_wifi.store(true, Ordering::Relaxed);

    info!(
        "Wifi DHCP info: {:?} quad {}",
        ip_info,
        last_quad.load(Ordering::Relaxed)
    );

    Ok(Box::new(esp_wifi))
}

pub fn create_wifi(
    ssid: &'static str,
    password: &'static str,
    has_wifi: &AtomicBool,
    last_quad: &AtomicI32,
    display_tx: Sender<DrawCmd>,
    modem: Modem,
    sysloop: EspEventLoop<System>,
    has_time: &AtomicBool,
) -> Result<()> {
    // display a message while searching for WiFi
    display_tx.send(DrawCmd::Text {
        pos: DrawPos::Pos(Point::new(10, 20)),
        font: Some(PROFONT_24_POINT),
        text: "Looking for WiFi".into(),
        text_color: RgbColor::BLACK,
        background: Some(RgbColor::WHITE),
    })?;
    let wifi = wifi(ssid, password, has_wifi, last_quad, modem, sysloop)?;
    let ip_info = wifi.sta_netif().get_ip_info()?;

    // clear the message area
    display_tx.send(DrawCmd::Clear {
        color: RgbColor::WHITE,
        pos: DrawPos::Box(Rectangle::new(Point::new(0, 0), Size::new(400, 30))),
    })?;

    // display a message with the IP address while waiting for SNTP
    display_tx.send(DrawCmd::Text {
        pos: DrawPos::Pos(Point::new(10, 22)),
        font: None,
        text: format!("IP Addr {}, SNTP init", ip_info.ip),
        text_color: RgbColor::BLACK,
        background: Some(RgbColor::WHITE),
    })?;
    let mut sntp_reset_cnt = 0;

    let _sntp = sntp::EspSntp::new_default()?;

    info!("SNTP initialized");

    let mut not_sync = true;
    loop {
        std::thread::sleep(Duration::from_secs(7));
        if not_sync {
            let status: SyncStatus = _sntp.get_sync_status();
            match status {
                SyncStatus::Completed => {
                    has_time.store(true, Ordering::Relaxed);
                    not_sync = false;
                }
                SyncStatus::InProgress => {
                    info!("Sync in progress");
                }
                SyncStatus::Reset => {
                    info!("SNTP reset");
                    sntp_reset_cnt += 1;
                    // if we're struggling to get the SNTP stuff set up
                    // after 700 seconds (> 10 minutes), reset the box
                    if sntp_reset_cnt > 100 {
                        esp_idf_hal::reset::restart();
                    }
                }
            }
        }
    }
}

// make a REST request on Home Assistant's API to get the state of
// a particular item
pub fn get_ha_state(item: &str, ha_url: &str, ha_headers: &[(&str, &str)]) -> Result<JsonValue> {
    use embedded_svc::http::client::*;
    use embedded_svc::utils::io;
    use esp_idf_svc::http::client::*;

    let mut client = Client::wrap(EspHttpConnection::new(&Configuration {
        crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),

        ..Default::default()
    })?);

    let full_url = format!("http://{}/api/states/{}", ha_url, item);

    let mut response = client
        .request(Method::Get, &full_url, ha_headers)?
        .submit()?;

    if response.status() != 200 {
        bail!(format!(
            "Request for {} yielded {}",
            item,
            response.status()
        ));
    }

    let mut source: Vec<u8> = vec![];
    let mut body = [0_u8; 512];

    loop {
        let read = io::try_read_full(&mut response, &mut body).map_err(|err| err.0)?;
        if read == 0 {
            break;
        }
        source.extend_from_slice(&body[0..read]);
    }

    let json = json::parse(&String::from_utf8_lossy(&source))?;

    Ok(json)
}
