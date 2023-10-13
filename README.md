# ESP32 S3 Box Lite & Home Assistant & Rust == Homer

The [ESP32 S3 Box Lite](https://www.adafruit.com/product/5511) is an absolutely
amazing piece of computing hardware. It's got roughly the specs of a 1983
[Lisa](https://en.wikipedia.org/wiki/Apple_Lisa) for $35USD. That's roughly
1/1000th of the price of the Lisa (inflation adjusted).

Having a powerful computer with built-in networking in a convenient box
with display and some buttons just begs to be programmed.

[Home Assistant](https://www.home-assistant.io/) is a privacy-focused home
automation tool that has a well designed API and a ton of functionality.

[Rust](https://www.rust-lang.org/) is a great language for building
(relatively) compact code. And there's increasingly solid support for [ESP32
in Rust](https://github.com/esp-rs/esp-idf-hal).

This project is a Rust-based system that does basic UI and actions
with Home Assistant on the ESP32 S3 Box Lite.

## Setup

Here are the Linux (should work on OSX) steps to build and enhance Homer.

### Install Rust

Follow the [Rust Up](https://rustup.rs/) instructions to install Rust on your local
machine.

You may also want to check out [Rust on the ESP32](https://esp-rs.github.io/book/).

### Install Rust for the ESP32 (both RISC-V and Xtensa)

The Box Lite uses the [S3](https://www.espressif.com/en/products/socs/esp32-s3) version
of the ESP32. The S3 is an Xtensa chip. There is not yet native support in Rust for the
Xtensa instruction set, so there's a bit of extra "stuff" that needs to be installed and
configured.

Crib notes from the [ESP32 RISC-V & Xtensa](https://esp-rs.github.io/book/installation/riscv-and-xtensa.html) installation page:

* `cargo install espup`
* `espup install`
* `. $HOME/export-esp.sh` (this must be done in each shell that does compilation or launches [VS Code](https://code.visualstudio.com/))

### Set Environment Variables

You'll be hardcoding the WiFi information and other stuff into the binaries you install
on the Box Lite.

The contents of these variables are inserted into the executable at compile time via the
Rust [`env!`](https://doc.rust-lang.org/std/macro.env.html) macro.

Set the following environment variables to appropriate values:

* `HOMER_SSID` -- The SSID of the WiFi network the device will be communicating with. Note that the ESP32 is 2.4Ghz only.
* `HOMER_WIFI_PASSWORD` -- The WiFi password
* `HOMER_TZ` -- The [time zone](https://www.gnu.org/software/libc/manual/html_node/TZ-Variable.html) where the device will be running. For me (I live near Boston) it's `EST+5EDT,M3.2.0/2,M11.1.0/2`
* `HOMER_HA_AUTH` -- The [Home Assistant authentication token](https://developers.home-assistant.io/docs/auth_api/#long-lived-access-token)
* `HOMER_HA_URL` -- The host and port of the Home Assistant instance. Note that `homeassistant.local` will *not* work as the ESP32 doesn't implement [Avahi](https://en.wikipedia.org/wiki/Avahi_%28software%29). I recommend using the IP address of your HA server. In my case it's `192.168.17.131:8123`.

### Doing the first build

Connect your Box Lite via USB to your computer.

From the command line (please remember you must do `. $HOME/export-esp.sh` in each
new terminal) issue the command to do the build and flash:  
`cargo espflash flash --monitor`

The first build will take a while (5+ minutes).

Note the `cargo run` does not work as part of the build requires setting the
[partition table](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/partition-tables.html)
on the device and for some reason, `cargo run` does not set the parition table, but 
`cargo espflash flash --monitor` does.

Assuming the build and flash worked correctly, the Box Lite should find your WiFi network and display
a clock (after doing an [SNTP](https://en.wikipedia.org/wiki/Network_Time_Protocol) sync). It will
also display "Failed to load config" as we don't have any configuration files.

### Flash the default configuration

The configuration for the Box Lite is stored in a [Spiffs](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/spiffs.html)
parition on the device.

To flash the configuration:
* `python3 spiffsgen.py 0x100000 configs target/configs.data` -- generate the spiffs filesystem from the files in the `configs` directory
* `espflash write-bin 0x310000 target/configs.data` -- put the filesystem on the Box Lite. It will reboot and display a message in blue about failing to find the config file for your device.

### Get the MAC address of the device

You can create a unique configuration for each of your Box Lite devices and the configuration
is loaded at boot time based on the MAC address of the device. Thus you pre-load the same
set of configurations onto the device and the device selects the configuration based on MAC
address.

To get the MAC address of the device from the command prompt: `espflash board-info`

### Create a config file for your device

Create a file based on the last 3 hex digits of the MAC address. For example, if the
MAC address of your device is `f4:12:fa:22:33:44`, the file to create in the `configs`
directory is `22_33_44.json`. These files are in `.gitignore` so that you don't accidentally
commit the files to your repo (they may contain sensitive information about your
Home Assistant configuration).

The JSON file should look something like:

```json
[
  {
    "Text": {
      "line": 0,
      "text": "Kitchen",
      "color": 31
    }
  },
  {
    "Line": {
      "line": 1,
      "ha_id": "sensor.outside_temperature",
      "text": "Outside Temp ",
      "make_int": true,
      "color": 63488
    }
  },
  {
    "Line": {
      "line": 2,
      "ha_id": "sensor.kitchen_temperature",
      "text": "Inside Temp ",
      "make_int": true,
      "color": 63519
    }
  },
  {
    "Button": {
      "button": 0,
      "ha_id": "light.kitchen_light",
      "cmp": {
        "Str": "on"
      },
      "text_on": "Dark",
      "text_off": "Light",
      "action_on": {
        "Scene": "scene.kitchen_on"
      },
      "action_off": {
        "Scene": "scene.kitchen_off"
      },
      "color": 31
    }
  }
]
```

These blocks correspond to the `HAConnect` enum:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HAConnect {
    Text {
        line: u8,
        text: String,
        color: u16,
    },
    Button {
        button: u8,
        ha_id: String,
        cmp: CmpValue,
        text_on: String,
        text_off: String,
        action_on: HAAction,
        action_off: HAAction,
        color: u16,
    },
    Line {
        line: u8,
        ha_id: String,
        text: String,
        make_int: bool,
        color: u16,
    },
}
```

`Text` is a line of text.

The `line` value is a zero-based line where the non-Button item is displayed.

`color` is an [Rgb565](https://rgbcolorpicker.com/565) colr value. Here are some helpful constants:

```
    // colors
    // red  0xf800 63488
    // green  0x7e0 2016
    // blue  0x1f 31
    // magenta  0xf81f 63519
    // yellow  0xffe3 65507
    // cyan  0x7ff 2047
    // black 0x0 0
    // white 0xffff 65535
```
For `Button`:
* `button` is 0 - 2 corresponding to the three buttons on the Box Lite
* `ha_id` is the id of the Home Assistent entity that's used to test the value of the button
* `cmp` is the value to compare against. It supports `f64` and `i64` comparision as well as `String` although
  I haven't found a use for anything but `String`
* `text_on` the text to display above the button when the `cmp` value matches the `ha_id`'s state ("on")
* `text_off` the text to display above the button when the "thing" the button refers to is not "on"
* `action_on` the action to take (the HA Scene the set) when the button is pushed and the state is not "on"
* `action_off` the action to take when the button is pressed and the state is "on"

For `Line`:
* `ha_id` the Home Assistant entity value to append to `text`
* `make_int` convert the entity state string to an int (rounded float) for display

Please remember to do the `python3 spiffsgen.py 0x100000 configs target/configs.data` and `espflash write-bin 0x310000 target/configs.data`
steps each time you make a configuration change.

### Have fun

That's the basic stuff you have to do to get an ESP32 S3 Box Lite system running Homer
and specifying configuration for each Box Lite.

The next section (not yet written) will describe the design of Homer and maybe invites
you to make a pull request.

## Architecture