use esp_hal::{
    Blocking,
    analog::adc::{Adc, AdcCalCurve, AdcCalLine, AdcConfig, AdcPin, Attenuation},
    gpio::{DriveMode, Level, Output, OutputConfig, Pull},
    peripherals::{ADC1, ADC2, GPIO1, GPIO4, GPIO11, GPIO12, GPIO16, GPIO21},
};

/// ADC and GPIO handles — private to the sensors module.
pub(super) struct SensorHardware<'a> {
    pub(super) adc1: Adc<'a, ADC1<'a>, Blocking>,
    pub(super) adc2: Adc<'a, ADC2<'a>, Blocking>,
    pub(super) moisture_pin: AdcPin<GPIO11<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    pub(super) waterlevel_pin: AdcPin<GPIO12<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    pub(super) battery_pin: AdcPin<GPIO4<'a>, ADC1<'a>, AdcCalLine<ADC1<'a>>>,
    pub(super) moisture_power_pin: Output<'a>,
    pub(super) water_level_power_pin: Output<'a>,
    pub(super) dht11_pin: esp_hal::gpio::Flex<'a>,
}

/// Peripheral bundle passed from main.rs into the sensor task.
pub struct SensorPeripherals {
    pub dht11_digital_pin: GPIO1<'static>,
    pub battery_pin: GPIO4<'static>,
    pub moisture_power_pin: GPIO16<'static>,
    pub moisture_analog_pin: GPIO11<'static>,
    pub water_level_analog_pin: GPIO12<'static>,
    pub water_level_power_pin: GPIO21<'static>,
    pub adc1: ADC1<'static>,
    pub adc2: ADC2<'static>,
}

/// Initialize all sensor hardware from the peripheral bundle.
pub(super) async fn initialize_hardware(p: SensorPeripherals) -> SensorHardware<'static> {
    let mut adc2_config = AdcConfig::new();
    let moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let waterlevel_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.water_level_analog_pin, Attenuation::_11dB);
    let adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let battery_pin = adc1_config.enable_pin_with_cal(p.battery_pin, Attenuation::_11dB);
    let adc1 = Adc::new(p.adc1, adc1_config);

    let moisture_power_pin = Output::new(p.moisture_power_pin, Level::Low, OutputConfig::default());
    let water_level_power_pin =
        Output::new(p.water_level_power_pin, Level::Low, OutputConfig::default());

    // Setup DHT11 pin once — open-drain, no pull, input enabled
    let mut dht11_pin = Output::new(
        p.dht11_digital_pin,
        Level::High,
        OutputConfig::default()
            .with_drive_mode(DriveMode::OpenDrain)
            .with_pull(Pull::None),
    )
    .into_flex();
    dht11_pin.set_input_enable(true);

    SensorHardware {
        adc1,
        adc2,
        moisture_pin,
        waterlevel_pin,
        battery_pin,
        moisture_power_pin,
        water_level_power_pin,
        dht11_pin,
    }
}
