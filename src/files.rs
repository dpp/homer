use std::{
    ffi::CString,
    fs::File,
    io::{BufReader, Read},
};

use anyhow::{bail, Result};

use esp_idf_sys::{esp_vfs_spiffs_conf_t, esp_vfs_spiffs_register, ESP_ERR_NOT_FOUND, ESP_OK};

pub fn mount_spiffs() -> Result<()> {
    let spiffy = CString::new("/spiffy").expect("CString::new failed");
    let spiffland = CString::new("spiffland").expect("CString::new failed");

    let conf = esp_vfs_spiffs_conf_t {
        base_path: spiffy.as_ptr(),
        partition_label: spiffland.as_ptr(),
        max_files: 5,
        format_if_mount_failed: true,
    };
    let ret = unsafe { esp_vfs_spiffs_register(&conf) };

    match ret {
        ESP_OK => Ok(()),
        ESP_ERR_NOT_FOUND => bail!("The SPIFF partition was not found, error {}", ret),
        err => bail!("Mounting SPIFF failed {}", err),
    }
}

pub fn read_file(name: &str) -> Result<String> {
    let file = File::open(format!("/spiffy/{}", name))?;
    let mut buf_reader = BufReader::new(file);
    let mut contents = String::new();
    buf_reader.read_to_string(&mut contents)?;

    Ok(contents)
}
