use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::Dimensions;
use embedded_graphics::mono_font::iso_8859_1::FONT_10X20 as FONT;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::Drawable;
use embedded_hal::delay::DelayNs;
use embedded_text::alignment::HorizontalAlignment;
use embedded_text::style::{HeightMode, TextBoxStyleBuilder};
use embedded_text::TextBox;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::peripherals::{
    GPIO38, GPIO39, GPIO40, GPIO41, GPIO42, GPIO45, GPIO46, GPIO47, GPIO48, GPIO5, GPIO6, GPIO7,
    GPIO8, GPIO9,
};
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

pub struct Display<'a, D: DelayNs> {
    display: MipiDisplayWrapper<'a>,
    backlight: Output<'a>,
    delay: D,
}

pub trait DisplayTrait {
    fn write_multiline(&mut self, text: &str) -> Result<(), Error>;
    fn enable_powersave(&mut self) -> Result<(), Error>;
}

pub struct DisplayPeripherals {
    pub rst: GPIO5<'static>,
    pub cs: GPIO6<'static>,
    pub dc: GPIO7<'static>,
    pub wr: GPIO8<'static>,
    pub rd: GPIO9<'static>,
    pub backlight: GPIO38<'static>,
    pub d0: GPIO39<'static>,
    pub d1: GPIO40<'static>,
    pub d2: GPIO41<'static>,
    pub d3: GPIO42<'static>,
    pub d4: GPIO45<'static>,
    pub d5: GPIO46<'static>,
    pub d6: GPIO47<'static>,
    pub d7: GPIO48<'static>,
}

impl<D: DelayNs> Display<'_, D> {
    pub fn new(p: DisplayPeripherals, mut delay: D) -> Result<Self, Error> {
        let backlight = Output::new(p.backlight, Level::Low, OutputConfig::default());

        let dc = Output::new(p.dc, Level::Low, OutputConfig::default());
        let mut cs = Output::new(p.cs, Level::Low, OutputConfig::default());
        let rst = Output::new(p.rst, Level::Low, OutputConfig::default());
        let wr = Output::new(p.wr, Level::Low, OutputConfig::default());
        let mut rd = Output::new(p.rd, Level::Low, OutputConfig::default());

        cs.set_low();
        rd.set_high();

        let d0 = Output::new(p.d0, Level::Low, OutputConfig::default());
        let d1 = Output::new(p.d1, Level::Low, OutputConfig::default());
        let d2 = Output::new(p.d2, Level::Low, OutputConfig::default());
        let d3 = Output::new(p.d3, Level::Low, OutputConfig::default());
        let d4 = Output::new(p.d4, Level::Low, OutputConfig::default());
        let d5 = Output::new(p.d5, Level::Low, OutputConfig::default());
        let d6 = Output::new(p.d6, Level::Low, OutputConfig::default());
        let d7 = Output::new(p.d7, Level::Low, OutputConfig::default());

        let bus = Generic8BitBus::new((d0, d1, d2, d3, d4, d5, d6, d7));

        let di = ParallelInterface::new(bus, dc, wr);

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

impl<D: DelayNs> DisplayTrait for Display<'_, D> {
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

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::DisplayInterface(msg) => write!(f, "Display interface error: {msg}"),
            Error::InitError => write!(f, "Display initialization error"),
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
