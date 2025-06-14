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
        HOMEASSISTANT_SENSOR_TOPIC, HOMEASSISTANT_VALVE_TOPIC,
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

enum MqttAction {
    None,
    ClearRetained(String),
}

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

    // Outer loop for handling reconnections
    'reconnect: loop {
        let mut client = match initialize_mqtt_client(stack, resources).await {
            Ok(client) => client,
            Err(e) => {
                println!("Error initializing MQTT client: {:?}. Retrying in 5s...", e);
                Timer::after(Duration::from_secs(5)).await;
                continue 'reconnect; // Retry connection
            }
        };

        let pump_set_topic = format!("{}/pump/set", DEVICE_ID);

        if let Err(e) = client.subscribe_to_topic(&pump_set_topic).await {
            println!(
                "Error subscribing to pump command topic: {}. Retrying connection...",
                e
            );
            Timer::after(Duration::from_secs(5)).await;
            continue 'reconnect; // Retry connection
        }

        println!("Subscribed to pump command topic: {}", pump_set_topic);

        // Inner loop for processing events while connected
        loop {
            let mut action_to_perform = MqttAction::None;

            match select(receiver.receive(), client.receive_message()).await {
                Either::First(sensor_data) => {
                    if let Err(e) = handle_sensor_data(&mut client, &mut display, sensor_data).await
                    {
                        println!("Error handling sensor data: {:?}. Reconnecting...", e);
                        continue 'reconnect; // Break inner loop, go to outer loop for reconnect
                    }
                }
                Either::Second(result) => match result {
                    Ok((topic, data)) => {
                        action_to_perform =
                            process_received_mqtt_message(topic, data, &pump_set_topic);
                    }
                    Err(e) => {
                        println!("Error receiving MQTT message: {}. Reconnecting...", e);
                        continue 'reconnect; // Break inner loop, go to outer loop for reconnect
                    }
                },
            }

            match action_to_perform {
                MqttAction::ClearRetained(topic_to_clear) => {
                    println!(
                        "Executing clear of retained message on topic: {}",
                        topic_to_clear
                    );
                    if let Err(e) = client
                        .send_message(&topic_to_clear, &[], QualityOfService::QoS0, true)
                        .await
                    {
                        println!("Error clearing retained message: {}. Reconnecting...", e);
                        continue 'reconnect; // Break inner loop, go to outer loop for reconnect
                    }
                }
                MqttAction::None => {}
            }
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
    publish_discovery_topics(client, &sensor_data).await?;

    if sensor_data.publish {
        publish_sensor_data(client, &sensor_data).await?;
    } else {
        println!("skipping publishing to MQTT");
    }

    process_display(display, &sensor_data).await?;
    Ok(())
}

async fn publish_discovery_topics(
    client: &mut MqttClientImpl<'_>,
    sensor_data: &SensorData,
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
    Ok(())
}

async fn publish_sensor_data(
    client: &mut MqttClientImpl<'_>,
    sensor_data: &SensorData,
) -> Result<(), Error> {
    // check if we can enable the pump
    let allow_enable_pump = sensor_data
        .data
        .iter()
        .any(|entry| matches!(entry, Sensor::WaterLevel(WaterLevel::Empty)));

    sensor_data.data.iter().for_each(|entry| {
        if let Sensor::PumpTrigger(enabled) = entry {
            let enabled = *enabled;
            if allow_enable_pump {
                println!("Pump trigger value: {} - updating pump state", enabled);
                update_pump_state(enabled);
            } else {
                update_pump_state(false);
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

    Ok(())
}

async fn process_display(
    display: &mut Display<'static, Delay>,
    sensor_data: &SensorData,
) -> Result<(), Error> {
    display.write_multiline(&format!("{}", sensor_data))?;
    Timer::after(Duration::from_secs(AWAKE_DURATION_SECONDS)).await;
    display.enable_powersave()?;
    Ok(())
}

fn process_received_mqtt_message(topic: &str, data: &[u8], pump_set_topic: &str) -> MqttAction {
    let msg = str::from_utf8(data).ok();
    let mut action = MqttAction::None;

    if let Some(message) = msg {
        if topic == pump_set_topic {
            if message.is_empty() {
                println!("Received empty message on '{}', likely the cleared retained message. Ignoring.", topic);
            } else {
                let state = message == "OPEN";
                println!("Pump command received on '{}'. State: {}", topic, state);
                update_pump_state(state);

                if state {
                    println!("Scheduling clear of retained message on topic: {}", topic);
                    action = MqttAction::ClearRetained(topic.to_string());
                }
            }
        } else {
            println!("Message on unhandled topic: {}", topic);
        }
    } else {
        println!("Invalid UTF-8 message received on topic {}", topic);
    }
    action
}

pub fn update_pump_state(state: bool) {
    {
        ENABLE_PUMP.signal(state);
    }
}

fn get_sensor_discovery(s: &Sensor) -> (String, String) {
    let topic = s.topic();
    let mut payload = get_common_device_info(topic, s.name());
    payload["state_topic"] = json!(format!("{}/{}", DEVICE_ID, topic));
    payload["value_template"] = json!("{{ value_json.value }}");
    payload["state_class"] = json!("measurement");
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
    payload["command_topic"] = json!(format!("{}/{}/set", DEVICE_ID, topic));
    payload["payload_open"] = json!("OPEN");
    payload["retain"] = json!(true);
    payload["payload_close"] = json!("CLOSE");

    let discovery_topic = format!(
        "{}/{}/{}_{}/config",
        HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX, HOMEASSISTANT_VALVE_TOPIC, DEVICE_ID, topic
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
