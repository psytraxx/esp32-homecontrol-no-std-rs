use alloc::{
    format,
    string::{String, ToString},
};
use core::{num::NonZero, num::ParseIntError, str};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_net::{
    Stack,
    dns::{DnsQueryType, Error as DnsError},
    tcp::{ConnectError, TcpSocket},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Receiver};
use embassy_time::{Delay, Duration, Timer};
use log::{error, info, warn};
use rust_mqtt::{
    Bytes,
    buffer::AllocBuffer,
    client::{
        Client, MqttError,
        event::Event,
        options::{
            ConnectOptions, PublicationOptions, RetainHandling, SubscriptionOptions, TopicReference,
        },
    },
    config::{KeepAlive, SessionExpiryInterval},
    types::{MqttBinary, MqttString, QoS, ReasonCode, TopicName},
};
use serde_json::{Value, json};
use static_cell::StaticCell;
use strum::IntoEnumIterator;

use crate::{
    DISCOVERY_MESSAGES_SENT, DISPLAY_SLEEP, ENABLE_PUMP,
    config::{
        DEVICE_ID, HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX, HOMEASSISTANT_SENSOR_TOPIC,
        HOMEASSISTANT_SWITCH_TOPIC, MQTT_PUBLISH_ENABLED,
    },
    display::{self, Display, DisplayTrait},
    domain::{Sensor, SensorData},
};

const BUFFER_SIZE: usize = 4096;

struct MqttResources {
    rx_buffer: [u8; BUFFER_SIZE],
    tx_buffer: [u8; BUFFER_SIZE],
    alloc_buffer: AllocBuffer,
}

static RESOURCES: StaticCell<MqttResources> = StaticCell::new();

#[embassy_executor::task]
pub async fn update_task(
    stack: Stack<'static>,
    mut display: Display<'static, Delay>,
    sensordata_receiver: Receiver<'static, NoopRawMutex, SensorData, 3>,
) {
    let resources = MqttResources {
        rx_buffer: [0u8; BUFFER_SIZE],
        tx_buffer: [0u8; BUFFER_SIZE],
        alloc_buffer: AllocBuffer,
    };

    let resources = RESOURCES.init(resources);

    // Outer loop for handling reconnections
    'reconnect: loop {
        let mut client = match initialize_mqtt_client(stack, resources).await {
            Ok(client) => client,
            Err(e) => {
                error!("Error initializing MQTT client: {:?}. Retrying in 5s...", e);
                Timer::after(Duration::from_secs(5)).await;
                continue 'reconnect;
            }
        };

        let pump_set_topic = format!("{DEVICE_ID}/pump/set");

        // Phase 1: wait for first sensor reading to know overflow state before
        // subscribing to the pump topic. This avoids the race where a retained ON
        // arrives before we know whether the pump is safe to run.
        let pump_allowed = match select(sensordata_receiver.receive(), DISPLAY_SLEEP.wait()).await {
            Either::First(sensor_data) => {
                let allowed = !sensor_data
                    .data
                    .iter()
                    .any(|e| matches!(e, Sensor::OverflowDetected(true)));

                if let Err(e) = handle_sensor_data(&mut client, &mut display, sensor_data).await {
                    error!("Error handling sensor data: {:?}. Reconnecting...", e);
                    continue 'reconnect;
                }
                allowed
            }
            Either::Second(_) => {
                info!("Display sleep signal received");
                if let Err(e) = display.enable_powersave() {
                    error!("Error enabling display powersave: {:?}", e);
                }
                return;
            }
        };

        // Phase 2: now subscribe — retained ON arrives with correct overflow state known.
        let sub_options = SubscriptionOptions {
            // Always deliver retained message on subscribe so a pending ON
            // set while the device was asleep is never missed.
            retain_handling: RetainHandling::AlwaysSend,
            retain_as_published: false,
            no_local: false,
            qos: QoS::AtMostOnce,
            ..Default::default()
        };

        let topic =
            TopicName::new_unchecked(MqttString::try_from(pump_set_topic.as_str()).unwrap());
        if let Err(e) = client.subscribe(topic.into(), sub_options).await {
            error!(
                "Error subscribing to pump command topic: {:?}. Retrying connection...",
                e
            );
            Timer::after(Duration::from_secs(5)).await;
            continue 'reconnect;
        }

        info!("Subscribed to pump command topic: {}", pump_set_topic);

        // Inner loop: sensor data updates pump_allowed; MQTT delivers pump commands.
        let mut pump_allowed = pump_allowed;
        loop {
            match select3(
                sensordata_receiver.receive(),
                client.poll(),
                DISPLAY_SLEEP.wait(),
            )
            .await
            {
                Either3::First(sensor_data) => {
                    pump_allowed = !sensor_data
                        .data
                        .iter()
                        .any(|e| matches!(e, Sensor::OverflowDetected(true)));

                    if let Err(e) = handle_sensor_data(&mut client, &mut display, sensor_data).await
                    {
                        error!("Error handling sensor data: {:?}. Reconnecting...", e);
                        continue 'reconnect;
                    }
                }
                Either3::Second(result) => match result {
                    Ok(Event::Publish(e)) => {
                        if let Err(e) = process_pump_command(
                            &mut client,
                            e.topic.as_ref().as_str(),
                            e.message.as_ref(),
                            &pump_set_topic,
                            pump_allowed,
                        )
                        .await
                        {
                            error!("Error processing pump command: {:?}. Reconnecting...", e);
                            continue 'reconnect;
                        }
                    }
                    Ok(e) => info!("Received event {:?}", e),
                    Err(e) => {
                        error!("Error receiving MQTT message: {:?}. Reconnecting...", e);
                        continue 'reconnect;
                    }
                },
                Either3::Third(_) => {
                    info!("Display sleep signal received");
                    if let Err(e) = display.enable_powersave() {
                        error!("Error enabling display powersave: {:?}", e);
                    }
                    return;
                }
            }
        }
    }
}

type MqttClientImpl<'a> = Client<'a, TcpSocket<'a>, AllocBuffer, 1, 1, 1, 1>;

async fn initialize_mqtt_client<'a>(
    stack: Stack<'static>,
    resources: &'a mut MqttResources,
) -> Result<MqttClientImpl<'a>, Error> {
    let mut socket = TcpSocket::new(stack, &mut resources.rx_buffer, &mut resources.tx_buffer);

    let host_addr = stack
        .dns_query(env!("MQTT_HOSTNAME"), DnsQueryType::A)
        .await
        .map(|a| a[0])?;

    let port = env!("MQTT_PORT").parse()?;
    let socket_addr = (host_addr, port);

    info!("Connecting to MQTT server...");
    socket.connect(socket_addr).await?;
    info!("Connected to MQTT server");

    let options = ConnectOptions {
        user_name: Some(MqttString::try_from(env!("MQTT_USERNAME")).unwrap()),
        password: Some(MqttBinary::try_from(env!("MQTT_PASSWORD")).unwrap()),
        clean_start: true,
        keep_alive: KeepAlive::Seconds(NonZero::new(60).unwrap()),
        session_expiry_interval: SessionExpiryInterval::Seconds(60),
        will: None,
        ..Default::default()
    };

    let mut client = Client::<'_, _, _, 1, 1, 1, 1>::new(&mut resources.alloc_buffer);

    match client
        .connect(
            socket,
            &options,
            Some(MqttString::try_from(DEVICE_ID).unwrap()),
        )
        .await
    {
        Ok(c) => {
            info!("Connected to server {:?}", c);
            info!("{:?}", client.client_config());
            info!("{:?}", client.server_config());
            info!("{:?}", client.shared_config());
            info!("{:?}", client.session());
        }
        Err(e) => {
            error!("Failed to connect to server: {:?}", e);
            return Err(e.into());
        }
    };

    info!("MQTT Broker connected");

    Ok(client)
}

async fn handle_sensor_data(
    client: &mut MqttClientImpl<'_>,
    display: &mut Display<'static, Delay>,
    sensor_data: SensorData,
) -> Result<(), Error> {
    if MQTT_PUBLISH_ENABLED {
        publish_discovery_topics(client).await?;
        publish_sensor_data(client, &sensor_data).await?;
    } else {
        info!("MQTT publishing disabled, skipping");
    }

    process_display(display, &sensor_data).await?;
    Ok(())
}

async fn publish_discovery_topics(client: &mut MqttClientImpl<'_>) -> Result<(), Error> {
    if !DISCOVERY_MESSAGES_SENT.get() {
        info!("First run, sending discovery messages");

        for s in Sensor::iter() {
            let (discovery_topic, message) = get_sensor_discovery(&s);

            let topic_ref = TopicReference::Name(TopicName::new_unchecked(
                MqttString::try_from(discovery_topic.as_str()).unwrap(),
            ));
            let options = PublicationOptions::new(topic_ref).retain();

            client
                .publish(&options, Bytes::Borrowed(message.as_bytes()))
                .await?;
            info!("Discovery message sent for sensor: {}", s.name());
        }

        for (discovery_topic, message) in [get_pump_switch_discovery()] {
            let topic_ref = TopicReference::Name(TopicName::new_unchecked(
                MqttString::try_from(discovery_topic.as_str()).unwrap(),
            ));
            let options = PublicationOptions::new(topic_ref).retain();
            client
                .publish(&options, Bytes::Borrowed(message.as_bytes()))
                .await?;
        }

        DISCOVERY_MESSAGES_SENT.set(true);
    } else {
        info!("Discovery messages already sent");
    }
    Ok(())
}

async fn process_pump_command(
    client: &mut MqttClientImpl<'_>,
    topic: &str,
    data: &[u8],
    pump_set_topic: &str,
    pump_allowed: bool,
) -> Result<(), Error> {
    if topic != pump_set_topic {
        warn!("Message on unhandled topic: {}", topic);
        return Ok(());
    }
    let Ok(message) = str::from_utf8(data) else {
        warn!("Invalid UTF-8 message on topic {}", topic);
        return Ok(());
    };
    match message {
        "ON" => {
            // Reset the switch immediately so HA reflects the outcome,
            // and a second wake doesn't re-trigger the pump.
            reset_pump_switch(client).await?;
            if pump_allowed {
                info!("Pump command received, starting pump");
                ENABLE_PUMP.signal(());
            } else {
                warn!("Pump command blocked: overflow detected");
            }
        }
        "OFF" => {} // broker echo after our own reset — ignore
        _ => warn!("Unexpected payload on '{}': {}", topic, message),
    }
    Ok(())
}

async fn reset_pump_switch(client: &mut MqttClientImpl<'_>) -> Result<(), Error> {
    let topic_name = format!("{DEVICE_ID}/pump/set");
    let topic_ref = TopicReference::Name(TopicName::new_unchecked(
        MqttString::try_from(topic_name.as_str()).unwrap(),
    ));
    let options = PublicationOptions::new(topic_ref).retain();
    client.publish(&options, Bytes::Borrowed(b"OFF")).await?;
    Ok(())
}

async fn publish_sensor_data(
    client: &mut MqttClientImpl<'_>,
    sensor_data: &SensorData,
) -> Result<(), Error> {
    for s in &sensor_data.data {
        let key = s.topic();
        let value = s.value();
        let message = json!({ "value": value }).to_string();
        let topic_name = format!("{DEVICE_ID}/{key}");

        info!(
            "Publishing to topic {}, message: {}",
            topic_name.as_str(),
            message.as_str()
        );

        let topic_ref = TopicReference::Name(TopicName::new_unchecked(
            MqttString::try_from(topic_name.as_str()).unwrap(),
        ));
        let options = PublicationOptions::new(topic_ref);

        client
            .publish(&options, Bytes::Borrowed(message.as_bytes()))
            .await?;
    }

    Ok(())
}

async fn process_display(
    display: &mut Display<'static, Delay>,
    sensor_data: &SensorData,
) -> Result<(), Error> {
    display.write_multiline(&format!("{sensor_data}"))?;
    Ok(())
}

fn get_sensor_discovery(s: &Sensor) -> (String, String) {
    let topic = s.topic();
    let mut payload = get_common_device_info(topic, s.name());
    payload["state_topic"] = json!(format!("{}/{}", DEVICE_ID, topic));
    payload["value_template"] = json!("{{ value_json.value }}");
    payload["platform"] = json!("sensor");
    payload["unique_id"] = json!(format!("{}_{}", DEVICE_ID, topic));

    let device_class = s.device_class();
    if let Some(device_class) = device_class {
        payload["device_class"] = json!(device_class);
    }

    let unit = s.unit();
    if let Some(unit) = unit {
        payload["unit_of_measurement"] = json!(unit);
        // only set state_class if unit is present - enables Home Assistant to display the unit correctly and keep track of state changes
        payload["state_class"] = json!("measurement");
        // force HA to record every incoming value even if unchanged (prevents recorder deduplication)
        payload["force_update"] = json!(true);
    }

    let discovery_topic = format!(
        "{HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX}/{HOMEASSISTANT_SENSOR_TOPIC}/{DEVICE_ID}_{topic}/config"
    );

    (discovery_topic, payload.to_string())
}

fn get_pump_switch_discovery() -> (String, String) {
    let mut payload = get_common_device_info("pump", "Water pump");
    payload["command_topic"] = json!(format!("{}/pump/set", DEVICE_ID));
    payload["state_topic"] = json!(format!("{}/pump/set", DEVICE_ID));
    payload["payload_on"] = json!("ON");
    payload["payload_off"] = json!("OFF");
    payload["retain"] = json!(true);

    let discovery_topic = format!(
        "{HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX}/{HOMEASSISTANT_SWITCH_TOPIC}/{DEVICE_ID}_pump/config"
    );
    (discovery_topic, payload.to_string())
}

fn get_common_device_info(topic: &str, name: &str) -> Value {
    json!({
        "name": name,
        "unique_id": format!("{}_{}", DEVICE_ID, topic),
        "device": {
            "identifiers": [DEVICE_ID],
            "name": "ESP32 Device",
            "model": "ESP32S3",
            "manufacturer": "Espressif"
        }
    })
}

#[derive(Debug)]
enum Error {
    Port,
    Dns(DnsError),
    Connection(ConnectError),
    Broker(ReasonCode),
    Display(display::Error),
    Mqtt,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Port => write!(f, "Port error"),
            Error::Dns(e) => write!(f, "DNS error: {e:?}"),
            Error::Connection(e) => write!(f, "Connection error: {e:?}"),
            Error::Broker(e) => write!(f, "Broker error: {e:?}"),
            Error::Display(e) => write!(f, "Display error: {e:?}"),
            Error::Mqtt => write!(f, "MQTT error"),
        }
    }
}

impl From<embassy_net::dns::Error> for Error {
    fn from(error: embassy_net::dns::Error) -> Self {
        Self::Dns(error)
    }
}

impl From<ConnectError> for Error {
    fn from(error: ConnectError) -> Self {
        Self::Connection(error)
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Self {
        Self::Port
    }
}

impl From<ReasonCode> for Error {
    fn from(error: ReasonCode) -> Self {
        Self::Broker(error)
    }
}

impl From<display::Error> for Error {
    fn from(error: display::Error) -> Self {
        Self::Display(error)
    }
}

impl<'a> From<MqttError<'a>> for Error {
    fn from(_error: MqttError<'a>) -> Self {
        Self::Mqtt
    }
}
