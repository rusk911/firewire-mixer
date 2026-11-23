pub mod bebob;
pub mod dice_eap;

use serde::Serialize;

use crate::transport::discovery::DeviceInfo;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceFamily {
    DiceEap,
    Bebob,
}

pub trait DeviceFamilyDriver: Sync {
    fn family(&self) -> DeviceFamily;
    fn supports(&self, device: &DeviceInfo) -> bool;
}

pub fn driver_for_device(device: &DeviceInfo) -> Option<&'static dyn DeviceFamilyDriver> {
    static DICE_EAP: dice_eap::Driver = dice_eap::Driver;
    static BEBOB: bebob::Driver = bebob::Driver;

    [&DICE_EAP as &dyn DeviceFamilyDriver, &BEBOB as &dyn DeviceFamilyDriver]
        .into_iter()
        .find(|driver| driver.supports(device))
}