use defmt::Format;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::{Dimensions, Point};
use embedded_graphics::mono_font::iso_8859_1::FONT_10X20 as FONT;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::text::{Baseline, Text};
use embedded_graphics::Drawable;
use embedded_text::alignment::HorizontalAlignment;
use embedded_text::style::{HeightMode, TextBoxStyleBuilder};
use embedded_text::TextBox;
use esp_hal::delay::Delay;
use esp_hal::gpio::{GpioPin, Level, Output};
use mipidsi::interface::{Generic8BitBus, ParallelError, ParallelInterface};
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation, Rotation};
use mipidsi::{Builder, Display as MipiDisplay};

use crate::config::{DISPLAY_HEIGHT, DISPLAY_WIDTH};

const TEXT_STYLE: MonoTextStyle<Rgb565> = MonoTextStyle::new(&FONT, Rgb565::WHITE);

type MipiDisplayWrapper<'a> = MipiDisplay<
    ParallelInterface<
        Generic8BitBus<
            Output<'a>,
            Output<'a>,
            Output<'a>,
            Output<'a>,
            Output<'a>,
            Output<'a>,
            Output<'a>,
            Output<'a>,
        >,
        Output<'a>,
        Output<'a>,
    >,
    ST7789,
    Output<'a>,
>;

pub struct Display<'a> {
    display: MipiDisplayWrapper<'a>,
    backlight: Output<'a>,
    delay: Delay,
}

pub trait DisplayTrait {
    fn write(&mut self, text: &str) -> Result<(), Error>;
    fn write_multiline(&mut self, text: &str) -> Result<(), Error>;
    fn enable_powersave(&mut self) -> Result<(), Error>;
}

pub struct DisplayPeripherals {
    pub rst: GpioPin<5>,
    pub cs: GpioPin<6>,
    pub dc: GpioPin<7>,
    pub wr: GpioPin<8>,
    pub rd: GpioPin<9>,
    pub backlight: GpioPin<38>,
    pub d0: GpioPin<39>,
    pub d1: GpioPin<40>,
    pub d2: GpioPin<41>,
    pub d3: GpioPin<42>,
    pub d4: GpioPin<45>,
    pub d5: GpioPin<46>,
    pub d6: GpioPin<47>,
    pub d7: GpioPin<48>,
}

impl<'a> Display<'a> {
    pub fn new(p: DisplayPeripherals) -> Result<Self, Error> {
        let backlight = Output::new(p.backlight, Level::Low);

        let dc = Output::new(p.dc, Level::Low);
        let mut cs = Output::new(p.cs, Level::Low);
        let rst = Output::new(p.rst, Level::Low);
        let wr = Output::new(p.wr, Level::Low);
        let mut rd = Output::new(p.rd, Level::Low);

        cs.set_low();
        rd.set_high();

        let d0 = Output::new(p.d0, Level::Low);
        let d1 = Output::new(p.d1, Level::Low);
        let d2 = Output::new(p.d2, Level::Low);
        let d3 = Output::new(p.d3, Level::Low);
        let d4 = Output::new(p.d4, Level::Low);
        let d5 = Output::new(p.d5, Level::Low);
        let d6 = Output::new(p.d6, Level::Low);
        let d7 = Output::new(p.d7, Level::Low);

        let bus = Generic8BitBus::new((d0, d1, d2, d3, d4, d5, d6, d7));

        let di = ParallelInterface::new(bus, dc, wr);

        let mut delay = Delay::new();

        let display = Builder::new(mipidsi::models::ST7789, di)
            .display_size(DISPLAY_HEIGHT, DISPLAY_WIDTH)
            .display_offset((240 - DISPLAY_HEIGHT) / 2, 0)
            .orientation(Orientation::new().rotate(Rotation::Deg270))
            .invert_colors(ColorInversion::Inverted)
            .reset_pin(rst)
            .init(&mut delay)
            .map_err(|_| Error::InitError)?;

        Ok(Self {
            display,
            backlight,
            delay,
        })
    }

    fn disable_powersave(&mut self) -> Result<(), Error> {
        self.backlight.set_high();
        self.display.wake(&mut self.delay)?;
        self.display.clear(RgbColor::BLACK)?;
        Ok(())
    }
}

impl<'a> DisplayTrait for Display<'a> {
    fn write(&mut self, text: &str) -> Result<(), Error> {
        self.disable_powersave()?;
        Text::with_baseline(text, Point::new(0, 0), TEXT_STYLE, Baseline::Top)
            .draw(&mut self.display)?;
        Ok(())
    }

    fn write_multiline(&mut self, text: &str) -> Result<(), Error> {
        self.disable_powersave()?;
        let textbox_style = TextBoxStyleBuilder::new()
            .height_mode(HeightMode::FitToText)
            .alignment(HorizontalAlignment::Justified)
            .build();

        // Create the text box and apply styling options.
        let text_box = TextBox::with_textbox_style(
            text,
            self.display.bounding_box(),
            TEXT_STYLE,
            textbox_style,
        );
        // Draw the text box.
        text_box.draw(&mut self.display)?;
        Ok(())
    }

    fn enable_powersave(&mut self) -> Result<(), Error> {
        self.backlight.set_low();
        self.display.sleep(&mut self.delay)?;
        Ok(())
    }
}

/// A clock error
#[derive(Debug)]
pub enum Error {
    DisplayInterface(&'static str),
    InitError,
}

impl Format for Error {
    fn format(&self, f: defmt::Formatter) {
        match self {
            Error::DisplayInterface(e) => defmt::write!(f, "Display error {}", e),
            Error::InitError => defmt::write!(f, "Init error"),
        }
    }
}

impl<BUS, DC, WR> From<ParallelError<BUS, DC, WR>> for Error {
    fn from(e: ParallelError<BUS, DC, WR>) -> Self {
        match e {
            ParallelError::Bus(_) => Self::DisplayInterface("Bus error"),
            ParallelError::Dc(_) => Self::DisplayInterface("Data/command pin error"),
            ParallelError::Wr(_) => Self::DisplayInterface("Write pin error"),
        }
    }
}
