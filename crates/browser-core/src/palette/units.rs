#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Length,
    Mass,
    Temperature,
    Volume,
    Data,
    Time,
}

/// Names a unit is typed as, its kind, and how many base units (metre, gram, litre, byte, second)
/// one of it is; a temperature's number is its scale for `celsius`.
const UNITS: [(&[&str], Kind, f64); 43] = [
    (&["mm", "millimeter", "millimeters", "millimetre", "millimetres"], Kind::Length, 0.001),
    (&["cm", "centimeter", "centimeters", "centimetre", "centimetres"], Kind::Length, 0.01),
    (&["m", "meter", "meters", "metre", "metres"], Kind::Length, 1.0),
    (&["km", "kilometer", "kilometers", "kilometre", "kilometres"], Kind::Length, 1000.0),
    (&["in", "inch", "inches", "\""], Kind::Length, 0.0254),
    (&["ft", "foot", "feet", "'"], Kind::Length, 0.3048),
    (&["yd", "yard", "yards"], Kind::Length, 0.9144),
    (&["mi", "mile", "miles"], Kind::Length, 1609.344),
    (&["nmi", "nautical mile", "nautical miles"], Kind::Length, 1852.0),
    (&["mg", "milligram", "milligrams"], Kind::Mass, 0.001),
    (&["g", "gram", "grams"], Kind::Mass, 1.0),
    (&["kg", "kilogram", "kilograms", "kilo", "kilos"], Kind::Mass, 1000.0),
    (&["t", "tonne", "tonnes"], Kind::Mass, 1_000_000.0),
    (&["oz", "ounce", "ounces"], Kind::Mass, 28.349_523_125),
    (&["lb", "lbs", "pound", "pounds"], Kind::Mass, 453.592_37),
    (&["st", "stone", "stones"], Kind::Mass, 6_350.293_18),
    (&["c", "°c", "celsius"], Kind::Temperature, 0.0),
    (&["f", "°f", "fahrenheit"], Kind::Temperature, 1.0),
    (&["k", "kelvin"], Kind::Temperature, 2.0),
    (&["ml", "milliliter", "milliliters", "millilitre", "millilitres"], Kind::Volume, 0.001),
    (&["l", "liter", "liters", "litre", "litres"], Kind::Volume, 1.0),
    (&["tsp", "teaspoon", "teaspoons"], Kind::Volume, 0.004_928_921_593_75),
    (&["tbsp", "tablespoon", "tablespoons"], Kind::Volume, 0.014_786_764_781_25),
    (&["floz", "fl oz", "fluid ounce", "fluid ounces"], Kind::Volume, 0.029_573_529_562_5),
    (&["cup", "cups"], Kind::Volume, 0.236_588_236_5),
    (&["pt", "pint", "pints"], Kind::Volume, 0.473_176_473),
    (&["qt", "quart", "quarts"], Kind::Volume, 0.946_352_946),
    (&["gal", "gallon", "gallons"], Kind::Volume, 3.785_411_784),
    (&["b", "byte", "bytes"], Kind::Data, 1.0),
    (&["kb", "kilobyte", "kilobytes"], Kind::Data, 1e3),
    (&["mb", "megabyte", "megabytes"], Kind::Data, 1e6),
    (&["gb", "gigabyte", "gigabytes"], Kind::Data, 1e9),
    (&["tb", "terabyte", "terabytes"], Kind::Data, 1e12),
    (&["kib", "kibibyte", "kibibytes"], Kind::Data, 1024.0),
    (&["mib", "mebibyte", "mebibytes"], Kind::Data, 1_048_576.0),
    (&["gib", "gibibyte", "gibibytes"], Kind::Data, 1_073_741_824.0),
    (&["ms", "millisecond", "milliseconds"], Kind::Time, 0.001),
    (&["s", "sec", "secs", "second", "seconds"], Kind::Time, 1.0),
    (&["min", "mins", "minute", "minutes"], Kind::Time, 60.0),
    (&["h", "hr", "hrs", "hour", "hours"], Kind::Time, 3600.0),
    (&["d", "day", "days"], Kind::Time, 86_400.0),
    (&["wk", "week", "weeks"], Kind::Time, 604_800.0),
    (&["yr", "year", "years"], Kind::Time, 31_557_600.0),
];

fn unit(name: &str) -> Option<(Kind, f64)> {
    let name = name.trim().to_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name);
    UNITS.iter().find(|(names, ..)| names.contains(&name)).map(|(_, kind, factor)| (*kind, *factor))
}

fn celsius(value: f64, scale: f64) -> f64 {
    match scale as u8 {
        1 => (value - 32.0) * 5.0 / 9.0,
        2 => value - 273.15,
        _ => value,
    }
}

fn from_celsius(value: f64, scale: f64) -> f64 {
    match scale as u8 {
        1 => value * 9.0 / 5.0 + 32.0,
        2 => value + 273.15,
        _ => value,
    }
}

/// An amount as typed: a comma before groups of exactly three digits, or anywhere beside a point,
/// separates thousands ("1,500", "1,500.5"); a single other comma is the decimal one ("1,5").
fn amount(text: &str) -> Option<f64> {
    let thousands = text.contains('.') || text.split(',').skip(1).all(|group| group.len() == 3);
    let text = match (thousands, text.matches(',').count()) {
        (true, _) => text.replace(',', ""),
        (false, 1) => text.replace(',', "."),
        _ => return None,
    };
    text.parse().ok()
}

/// "5 km in mi", "5km to miles", "100 f in c", "1,5 km in m", "12 in to cm": the amount converted,
/// with the target unit as typed; every " in ", " to " and " as " is tried until both units are
/// known. Units of different kinds, unknown units and text over `calc::MAX_INPUT` bytes give `None`.
pub fn convert(text: &str) -> Option<(f64, String)> {
    let text = text.trim();
    if text.len() > super::calc::MAX_INPUT {
        return None;
    }
    let lower = text.to_lowercase();
    [" in ", " to ", " as "]
        .iter()
        .flat_map(|sep| lower.match_indices(sep).map(|(at, sep)| (&lower[..at], &lower[at + sep.len()..])))
        .find_map(|(from, to)| convert_split(from, to))
}

fn convert_split(from: &str, to: &str) -> Option<(f64, String)> {
    let from = from.trim();
    let split = from.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == ',')).unwrap_or(from.len());
    let amount = amount(&from[..split])?;
    let (kind, factor) = unit(&from[split..])?;
    let (target, target_factor) = unit(to)?;
    if kind != target || !amount.is_finite() {
        return None;
    }
    let value = match kind {
        Kind::Temperature => from_celsius(celsius(amount, factor), target_factor),
        _ => amount * factor / target_factor,
    };
    value.is_finite().then(|| (value, to.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::calc::format;

    fn shown(text: &str) -> Option<String> {
        convert(text).map(|(value, unit)| format!("{} {unit}", format(value)))
    }

    #[test]
    fn converts_within_a_kind() {
        assert_eq!(shown("5 km in mi").as_deref(), Some("3.106855961 mi"));
        assert_eq!(shown("5km to miles").as_deref(), Some("3.106855961 miles"));
        assert_eq!(shown("1 lb in g").as_deref(), Some("453.59237 g"));
        assert_eq!(shown("212 F in C").as_deref(), Some("100 c"));
        assert_eq!(shown("0 c in k").as_deref(), Some("273.15 k"));
        assert_eq!(shown("-40 °c in °f").as_deref(), Some("-40 °f"));
        assert_eq!(shown("1 gal in l").as_deref(), Some("3.785411784 l"));
        assert_eq!(shown("1 GiB in MB").as_deref(), Some("1073.741824 mb"));
        assert_eq!(shown("2 hours in min").as_deref(), Some("120 min"));
        assert_eq!(shown("1 week in days").as_deref(), Some("7 days"));
        assert_eq!(shown("1,000 m in km").as_deref(), Some("1 km"));
    }

    #[test]
    fn reads_a_decimal_comma_and_thousands() {
        assert_eq!(shown("1,5 km in m").as_deref(), Some("1500 m"));
        assert_eq!(shown("1,500 km in m").as_deref(), Some("1500000 m"));
        assert_eq!(shown("1,500.5 m in km").as_deref(), Some("1.5005 km"));
        assert_eq!(shown("1,234,567 g in kg").as_deref(), Some("1234.567 kg"));
        assert_eq!(convert("1,5,6 km in m"), None);
    }

    #[test]
    fn tries_every_separator_until_both_units_are_known() {
        assert_eq!(shown("12 in to cm").as_deref(), Some("30.48 cm"));
        assert_eq!(shown("3 ft in in").as_deref(), Some("36 in"));
        assert_eq!(shown("10 min as s").as_deref(), Some("600 s"));
    }

    #[test]
    fn refuses_mixed_kinds_and_unknown_units() {
        for text in ["5 km in kg", "5 parsecs in km", "five km in mi", "5 km", "km in mi", "weather in berlin", "5 km in"] {
            assert_eq!(convert(text), None, "{text}");
        }
        assert_eq!(convert(&format!("1 {} in m", "k".repeat(300))), None);
    }
}
