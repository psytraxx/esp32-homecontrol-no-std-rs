use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
};

/// How long to wait for a pulse on the data line (in microseconds).
const TIMEOUT_US: u16 = 1_000;

/// Error type for this crate.
#[derive(Debug)]
pub enum Error<E> {
    /// Timeout during communication.
    Timeout,
    /// CRC mismatch.
    CrcMismatch,
    /// GPIO error.
    Gpio(E),
}

/// A DHT11 device.
pub struct Dht11<GPIO> {
    /// The concrete GPIO pin implementation.
    gpio: GPIO,
}

/// Results of a reading performed by the DHT11.
#[derive(Copy, Clone, Default, Debug)]
pub struct Measurement {
    /// The measured temperature.
    pub temperature: u8,
    /// The measured humidity in percent.
    pub humidity: u8,
}

impl<GPIO, E> Dht11<GPIO>
where
    GPIO: InputPin<Error = E> + OutputPin<Error = E>,
{
    /// Creates a new DHT11 device connected to the specified pin.
    pub fn new(gpio: GPIO) -> Self {
        Dht11 { gpio }
    }

    /// Performs a reading of the sensor.
    pub fn read<D>(&mut self, delay: &mut D) -> Result<Measurement, Error<E>>
    where
        D: DelayNs,
    {
        let mut data = [0u8; 5];

        // Perform initial handshake
        self.perform_handshake(delay)?;

        // Read bits
        for i in 0..40 {
            data[i / 8] <<= 1;
            if self.read_bit(delay)? {
                data[i / 8] |= 1;
            }
        }

        // Finally wait for line to go idle again.
        self.wait_for_pulse(true, delay)?;

        // Check CRC
        let crc = data[0]
            .wrapping_add(data[1])
            .wrapping_add(data[2])
            .wrapping_add(data[3]);
        if crc != data[4] {
            return Err(Error::CrcMismatch);
        }

        Ok(Measurement {
            temperature: data[1],
            humidity: data[0],
        })
    }

    fn perform_handshake<D>(&mut self, delay: &mut D) -> Result<(), Error<E>>
    where
        D: DelayNs,
    {
        // Set pin as floating to let pull-up raise the line and start the reading process.
        self.gpio.set_high().map_err(Error::Gpio)?;
        delay.delay_ms(1);

        // Pull line low for at least 18ms to send a start command.
        self.gpio.set_low().map_err(Error::Gpio)?;
        delay.delay_ms(20);

        // Restore floating
        self.gpio.set_high().map_err(Error::Gpio)?;
        delay.delay_us(40);

        // As a response, the device pulls the line low for 80us and then high for 80us.
        self.read_bit(delay)?;

        Ok(())
    }

    fn read_bit<D>(&mut self, delay: &mut D) -> Result<bool, Error<E>>
    where
        D: DelayNs,
    {
        let low = self.wait_for_pulse(true, delay)?;
        let high = self.wait_for_pulse(false, delay)?;
        Ok(high > low)
    }

    fn wait_for_pulse<D>(&mut self, level: bool, delay: &mut D) -> Result<u32, Error<E>>
    where
        D: DelayNs,
    {
        let mut count = 0;

        while self.gpio.is_high().map_err(Error::Gpio)? != level {
            count += 1;
            if count > TIMEOUT_US {
                return Err(Error::Timeout);
            }
            delay.delay_us(1);
        }

        Ok(u32::from(count))
    }
}
