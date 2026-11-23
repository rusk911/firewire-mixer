use super::{DeviceFamily, DeviceFamilyDriver};
use crate::transport::discovery::DeviceInfo;

pub struct Driver;

impl DeviceFamilyDriver for Driver {
    fn family(&self) -> DeviceFamily {
        DeviceFamily::Bebob
    }

    fn supports(&self, device: &DeviceInfo) -> bool {
        device.supported && device.protocol.as_deref() == Some("BeBoB")
    }
}