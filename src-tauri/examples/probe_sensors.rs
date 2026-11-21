//! Standalone sensor probe: tries to WAKE the ambient-light sensors by setting
//! SENSOR_PROPERTY_CURRENT_REPORT_INTERVAL, then polls lux for 15 s. Answers
//! whether the two AMS TCS3408 sensors are real-but-sleeping or genuinely
//! dead. One process, one backend (COM only), per the one-backend-per-process
//! rule in hw/display.rs.
use std::time::Duration;
use windows::Win32::Devices::PortableDevices::IPortableDeviceValues;
use windows::Win32::Devices::Sensors::{
    ISensorManager, SensorManager, SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX, SENSOR_DATA_TYPE_TIMESTAMP,
    SENSOR_PROPERTY_CURRENT_REPORT_INTERVAL, SENSOR_TYPE_AMBIENT_LIGHT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

fn pdv_with_interval(ms: u32) -> windows::core::Result<IPortableDeviceValues> {
    let values: IPortableDeviceValues = unsafe {
        CoCreateInstance(
            &windows::Win32::Devices::PortableDevices::PortableDeviceValues,
            None,
            CLSCTX_ALL,
        )
    }?;
    unsafe {
        values.SetValue(&SENSOR_PROPERTY_CURRENT_REPORT_INTERVAL, &ms.into())?;
    }
    Ok(values)
}

fn read_report(
    sensor: &windows::Win32::Devices::Sensors::ISensor,
) -> (Option<f64>, Option<String>) {
    unsafe {
        match sensor.GetData() {
            Ok(report) => {
                let lux = report
                    .GetSensorValue(&SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX)
                    .ok()
                    .and_then(|v| f64::try_from(&v).ok());
                let ts = report
                    .GetSensorValue(&SENSOR_DATA_TYPE_TIMESTAMP)
                    .ok()
                    .map(|v| format!("{v:?}"));
                (lux, ts)
            }
            Err(e) => (None, Some(format!("GetData failed: {e}"))),
        }
    }
}

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let manager: ISensorManager = CoCreateInstance(&SensorManager, None, CLSCTX_ALL)
            .expect("CoCreateInstance SensorManager");
        let collection = manager
            .GetSensorsByType(&SENSOR_TYPE_AMBIENT_LIGHT)
            .expect("GetSensorsByType");
        let count = collection.GetCount().unwrap_or(0);
        println!("COM: ambient-light sensor count = {count}");
        for i in 0..count {
            let Ok(sensor) = collection.GetAt(i) else {
                continue;
            };
            let (pre_lux, pre_ts) = read_report(&sensor);
            println!("COM: sensor[{i}] BEFORE lux={pre_lux:?} ts={pre_ts:?}");
            // Wake the sensor: a client-visible report interval tells the
            // driver stack this process wants fresh data every 1000 ms.
            match pdv_with_interval(1000) {
                Ok(values) => match sensor.SetProperties(&values) {
                    Ok(_) => println!("COM: sensor[{i}] SetProperties(REPORT_INTERVAL=1000) ok"),
                    Err(e) => println!("COM: sensor[{i}] SetProperties failed: {e}"),
                },
                Err(e) => println!("COM: sensor[{i}] PortableDeviceValues failed: {e}"),
            }
            for step in 1..=15 {
                std::thread::sleep(Duration::from_secs(1));
                let (lux, ts) = read_report(&sensor);
                println!("  +{step}s poll: lux={lux:?} ts={ts:?}");
            }
        }
    }
}
