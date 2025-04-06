use alloc::{
    format,
    string::{String, ToString},
};
use core::{num::ParseIntError, str};
use embassy_futures::select::{select, Either};
use embassy_net::{
    dns::{DnsQueryType, Error as DnsError},
    tcp::{ConnectError, TcpSocket},
    Stack,
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Receiver};
use embassy_time::{Delay, Duration, Timer};
use esp_println::println;
use rust_mqtt::{
    client::{
        client::MqttClient,
        client_config::{ClientConfig, MqttVersion::MQTTv5},
    },
    packet::v5::{publish_packet::QualityOfService, reason_codes::ReasonCode},
    utils::rng_generator::CountingRng,
};
use serde_json::{json, Value};
use static_cell::StaticCell;

use crate::{
    config::{
        AWAKE_DURATION_SECONDS, DEVICE_ID, HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX,
        HOMEASSISTANT_SENSOR_SWITCH, HOMEASSISTANT_SENSOR_TOPIC,
    },
    display::{self, Display, DisplayTrait},
    domain::{Sensor, SensorData, WaterLevel},
    DISCOVERY_MESSAGES_SENT, ENABLE_PUMP,
};

const BUFFER_SIZE: usize = 4096;
const BUFFER_SIZE_CLIENT: usize = 1024;

struct MqttResources {
    rx_buffer: [u8; BUFFER_SIZE],
    tx_buffer: [u8; BUFFER_SIZE],
    client_rx_buffer: [u8; BUFFER_SIZE_CLIENT],
    client_tx_buffer: [u8; BUFFER_SIZE_CLIENT],
}

static RESOURCES: StaticCell<MqttResources> = StaticCell::new();

#[embassy_executor::task]
pub async fn update_task(
    stack: Stack<'static>,
    mut display: Display<'static, Delay>,
    receiver: Receiver<'static, NoopRawMutex, SensorData, 3>,
) {
    let resources = MqttResources {
        rx_buffer: [0u8; BUFFER_SIZE],
        tx_buffer: [0u8; BUFFER_SIZE],
        client_rx_buffer: [0u8; BUFFER_SIZE_CLIENT],
        client_tx_buffer: [0u8; BUFFER_SIZE_CLIENT],
    };

    let resources = RESOURCES.init(resources);

    loop {
        let mut client = match initialize_mqtt_client(stack, resources).await {
            Ok(client) => client,
            Err(e) => {
                println!("Error initializing MQTT client: {:?}", e);
                continue;
            }
        };

        if let Err(e) = client
            .subscribe_to_topic("esp32_breadboard/pump/command")
            .await
        {
            println!("Error subscribing to pump command topic: {}", e);
            continue;
        }

        println!("Subscribed to pump command topic");

        match select(receiver.receive(), client.receive_message()).await {
            Either::First(sensor_data) => {
                if let Err(e) = handle_sensor_data(&mut client, &mut display, sensor_data).await {
                    println!("Error handling sensor data: {:?}", e);
                    continue;
                }
            }
            Either::Second(result) => match result {
                Ok((topic, data)) => {
                    handle_mqtt_message(topic, data);
                }
                Err(e) => {
                    println!("Error handling MQTT message: {}", e);
                    continue;
                }
            },
        }
    }
}

type MqttClientImpl<'a> = MqttClient<'a, TcpSocket<'a>, 5, CountingRng>;

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

    println!("Connecting to MQTT server...");
    socket.connect(socket_addr).await?;
    println!("Connected to MQTT server");

    println!("Initializing MQTT connection");
    let mut mqtt_config: ClientConfig<5, CountingRng> =
        ClientConfig::new(MQTTv5, CountingRng(20000));
    mqtt_config.add_username(env!("MQTT_USERNAME"));
    mqtt_config.add_password(env!("MQTT_PASSWORD"));
    mqtt_config.add_client_id(DEVICE_ID);

    let mut client = MqttClient::new(
        socket,
        &mut resources.client_tx_buffer,
        BUFFER_SIZE_CLIENT,
        &mut resources.client_rx_buffer,
        BUFFER_SIZE_CLIENT,
        mqtt_config,
    );

    client.connect_to_broker().await?;

    println!("MQTT Broker connected");

    Ok(client)
}

async fn handle_sensor_data(
    client: &mut MqttClientImpl<'_>,
    display: &mut Display<'static, Delay>,
    sensor_data: SensorData,
) -> Result<(), Error> {
    let discovery_messages_sent = unsafe { DISCOVERY_MESSAGES_SENT };
    if !discovery_messages_sent {
        println!("First run, sending discovery messages");
        for s in &sensor_data.data {
            let (discovery_topic, message) = get_sensor_discovery(s);
            client
                .send_message(
                    &discovery_topic,
                    message.as_bytes(),
                    QualityOfService::QoS0,
                    true,
                )
                .await?;
        }

        let (discovery_topic, message) = get_pump_discovery("pump");
        client
            .send_message(
                &discovery_topic,
                message.as_bytes(),
                QualityOfService::QoS0,
                true,
            )
            .await?;

        unsafe {
            DISCOVERY_MESSAGES_SENT = true;
        }
    } else {
        println!("Discovery messages already sent");
    }

    // act on sensor data - turn pump on/off
    sensor_data.data.iter().for_each(|entry| {
        if let Sensor::WaterLevel(WaterLevel::Full) = entry {
            // Water level is full, stop the pump in any case if it's running
            println!("Water level is full, stopping pump");
            update_pump_state(false);
        } else if let Sensor::PumpTrigger(enabled) = entry {
            // Pump trigger is enabled, start the pump
            if *enabled {
                println!("Soil moisture is low, starting pump");
                update_pump_state(true);
            }
        }
    });

    for s in &sensor_data.data {
        let key = s.topic();
        let value = s.value();
        let message = json!({ "value": value }).to_string();
        let topic_name = format!("{}/{}", DEVICE_ID, key);

        println!(
            "Publishing to topic {}, message: {}",
            topic_name.as_str(),
            message.as_str()
        );

        client
            .send_message(
                &topic_name,
                message.as_bytes(),
                QualityOfService::QoS0,
                false,
            )
            .await?;
    }

    display.write_multiline(&format!("{}", sensor_data))?;

    let pump_topic = format!("{}/pump/state", DEVICE_ID);
    let message = "OFF";
    println!(
        "Publishing to topic {}, message: {}",
        pump_topic.as_str(),
        message
    );

    client
        .send_message(
            &pump_topic,
            message.as_bytes(),
            QualityOfService::QoS0,
            false,
        )
        .await?;

    Timer::after(Duration::from_secs(AWAKE_DURATION_SECONDS)).await;

    display.enable_powersave()?;

    Ok(())
}

fn handle_mqtt_message(topic: &str, data: &[u8]) {
    let msg = str::from_utf8(data).ok();

    if let Some(message) = msg {
        println!("Received message: {:?} on topic {}", msg, topic);
        let state = message == "ON";
        println!("Pump state: {}", state);
        update_pump_state(state);
    } else {
        println!("Invalid message received on topic {}", topic);
    }
}

pub fn update_pump_state(state: bool) {
    // Use a scoped block so that the signal operation (which may internally acquire a mutex)
    // does not accidentally span long-running operations.
    {
        ENABLE_PUMP.signal(state);
    } // Critical section ends here immediately.
}

/// Get the MQTT discovery message for a sensor
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

    if let Sensor::WaterLevel(_) = s {
        payload["payload_on"] = json!(WaterLevel::Full);
        payload["payload_off"] = json!(WaterLevel::Empty);
    }

    let unit = s.unit();
    if let Some(unit) = unit {
        payload["unit_of_measurement"] = json!(unit);
    }

    let discovery_topic = format!(
        "{}/{}/{}_{}/config",
        HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX, HOMEASSISTANT_SENSOR_TOPIC, DEVICE_ID, topic
    );

    (discovery_topic, payload.to_string())
}

fn get_pump_discovery(topic: &str) -> (String, String) {
    let mut payload = get_common_device_info(topic, "Pump");
    payload["state_topic"] = json!(format!("{}/{}/state", DEVICE_ID, topic));
    payload["command_topic"] = json!(format!("{}/{}/command", DEVICE_ID, topic));
    // TODO: availability
    payload["payload_on"] = json!("ON");
    payload["payload_off"] = json!("OFF");

    let discovery_topic = format!(
        "{}/{}/{}_{}/config",
        HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX, HOMEASSISTANT_SENSOR_SWITCH, DEVICE_ID, topic
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
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Port => write!(f, "Port error"),
            Error::Dns(e) => write!(f, "DNS error: {:?}", e),
            Error::Connection(e) => write!(f, "Connection error: {:?}", e),
            Error::Broker(e) => write!(f, "Broker error: {:?}", e),
            Error::Display(e) => write!(f, "Display error: {:?}", e),
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
