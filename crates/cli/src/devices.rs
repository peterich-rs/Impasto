//! Discover development targets without building an application.

use std::process::Command;

use anyhow::{Context, Result, ensure};

pub(crate) struct Android {
    pub(crate) serial: String,
    pub(crate) state: String,
    model: String,
}

pub(crate) fn android() -> Result<Vec<Android>> {
    let output = Command::new("adb")
        .args(["devices", "-l"])
        .output()
        .context("启动 adb")?;
    ensure!(
        output.status.success(),
        "adb devices 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    Ok(parse_android(&text))
}

fn parse_android(text: &str) -> Vec<Android> {
    text.lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let serial = words.next()?.to_owned();
            let state = words.next()?.to_owned();
            let model = words
                .find_map(|word| word.strip_prefix("model:"))
                .map_or_else(|| serial.clone(), |model| model.replace('_', " "));
            Some(Android {
                serial,
                state,
                model,
            })
        })
        .collect()
}

pub(crate) fn run() {
    println!("{:<10} {:<24} COMMAND", "PLATFORM", "STATUS");
    if cfg!(target_os = "macos") {
        println!("{:<10} {:<24} impasto run -d macos", "macOS", "local");
    }
    match android() {
        Ok(devices) => {
            for device in devices {
                let status = format!("{} ({})", device.model, device.state);
                println!(
                    "{:<10} {:<24} impasto run -d {}",
                    "Android", status, device.serial
                );
            }
        }
        Err(error) => eprintln!("impasto: Android 设备不可用。{error:#}"),
    }
    #[cfg(target_os = "macos")]
    {
        match crate::ios::simulators() {
            Ok(devices) => {
                for device in devices {
                    let status = format!("{} ({})", device.name, device.state);
                    println!(
                        "{:<10} {:<24} impasto run -d ios:{}",
                        "Simulator", status, device.udid
                    );
                }
            }
            Err(error) => eprintln!("impasto: iOS 模拟器不可用。{error:#}"),
        }
        let devices = crate::ios::physical_devices().unwrap_or_else(|error| {
            eprintln!("impasto: devicectl 设备列表不可用。{error:#}");
            Vec::new()
        });
        for device in &devices {
            let udid = if device.udid.is_empty() {
                &device.identifier
            } else {
                &device.udid
            };
            println!(
                "{:<10} {:<24} impasto run -d ios:{udid}",
                "iOS", device.name
            );
        }
        if let Ok(usb) = crate::usbmuxd::devices() {
            for device in usb.into_iter().filter(|usb| {
                !devices
                    .iter()
                    .any(|device| crate::usbmuxd::matches(usb, &device.udid))
            }) {
                println!(
                    "{:<10} {:<24} impasto run -d ios:{}",
                    "iOS", "USB", device.serial
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_android;

    #[test]
    fn reports_authorization_and_device_models() {
        let devices = parse_android(
            "List of devices attached\nemulator-5554 device model:Pixel_9 transport_id:1\n002 unauthorized usb:001\n\n",
        );
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].model, "Pixel 9");
        assert_eq!(devices[1].state, "unauthorized");
    }
}
