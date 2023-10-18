// Copyright 2023 Turing Machines
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use anyhow::{Context, Result};
use rusb::{Device, GlobalContext, UsbContext};
use std::{path::PathBuf, time::Duration};

use crate::firmware_update::FwUpdateError;

pub(crate) fn get_usb_devices<'a, I: IntoIterator<Item = &'a (u16, u16)>>(
    filter: I,
) -> std::result::Result<Vec<Device<GlobalContext>>, FwUpdateError> {
    let all_devices = rusb::DeviceList::new()?;
    let filter = filter.into_iter().collect::<Vec<&'a (u16, u16)>>();
    let devices = all_devices
        .iter()
        .filter_map(|dev| {
            let desc = dev.device_descriptor().ok()?;
            let this = (desc.vendor_id(), desc.product_id());
            filter.contains(&&this).then_some(dev)
        })
        .collect::<Vec<Device<GlobalContext>>>();

    log::debug!("matches:{:?}", devices);
    Ok(devices)
}

#[allow(dead_code)]
fn map_to_serial<T: UsbContext>(dev: &rusb::Device<T>) -> anyhow::Result<String> {
    let desc = dev.device_descriptor()?;
    let handle = dev.open()?;
    let timeout = Duration::from_secs(1);
    let language = handle.read_languages(timeout)?;
    handle
        .read_serial_number_string(language.first().copied().unwrap(), &desc, timeout)
        .context("error reading serial")
}

pub(crate) fn extract_one_device<T>(devices: &[T]) -> Result<&T, FwUpdateError> {
    match devices.len() {
        1 => Ok(devices.first().unwrap()),
        0 => Err(FwUpdateError::NoDevices),
        n => Err(FwUpdateError::MultipleDevicesFound(n)),
    }
}

pub async fn get_device_path<I: IntoIterator<Item = &'static str>>(
    allowed_vendors: I,
) -> Result<PathBuf, FwUpdateError> {
    let mut contents = tokio::fs::read_dir("/dev/disk/by-id")
        .await
        .map_err(|err| {
            std::io::Error::new(err.kind(), format!("Failed to list devices: {}", err))
        })?;

    let target_prefixes = allowed_vendors
        .into_iter()
        .map(|vendor| format!("usb-{}_", vendor))
        .collect::<Vec<String>>();

    let mut matching_devices = vec![];

    while let Some(entry) = contents.next_entry().await.map_err(|err| {
        std::io::Error::new(
            err.kind(),
            format!("Intermittent IO error while listing devices: {}", err),
        )
    })? {
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };

        for prefix in &target_prefixes {
            if file_name.starts_with(prefix) {
                matching_devices.push(file_name.clone());
            }
        }
    }

    // Exclude partitions, i.e. turns [ "x-part2", "x-part1", "x", "y-part2", "y-part1", "y" ]
    // into ["x", "y"].
    let unique_root_devices = matching_devices
        .iter()
        .filter(|this| {
            !matching_devices
                .iter()
                .any(|other| this.starts_with(other) && *this != other)
        })
        .collect::<Vec<&String>>();

    let symlink = match unique_root_devices[..] {
        [] => {
            return Err(FwUpdateError::NoMsdDevices);
        }
        [device] => device.clone(),
        _ => {
            return Err(FwUpdateError::MultipleDevicesFound(
                unique_root_devices.len(),
            ));
        }
    };

    Ok(tokio::fs::canonicalize(format!("/dev/disk/by-id/{}", symlink)).await?)
}
