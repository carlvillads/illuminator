use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Tracer half-life in minutes from <https://www.nndc.bnl.gov/nudat3/indx_dec.jsp>
pub const TRACERS: [(&str, f64); 9] = [
    ("F-18", 109.77),
    ("C-11", 20.3634),
    ("Ga-68", 67.71),
    ("O-15", 2.0374),
    ("N-13", 9.9584),
    ("Cu-64", 762.036),
    ("Zr-89", 4701.84),
    ("Rb-82", 1.2575),
    ("I-124", 6013.44),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuvInputs {
    pub weight_kg: f64,
    pub dose_mbq: f64,
    pub minutes_after_injection: f64,
    pub half_life_minutes: f64,
    pub bq_ml_per_unit: f64,
}

impl SuvInputs {
    pub fn factor(&self) -> Result<f64> {
        let positive = [
            self.weight_kg,
            self.dose_mbq,
            self.half_life_minutes,
            self.bq_ml_per_unit,
        ];
        if positive
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
            || !self.minutes_after_injection.is_finite()
        {
            return Err(Error::invalid(
                "SUV needs a positive weight, dose and half-life",
            ));
        }
        let decay = 0.5f64.powf(self.minutes_after_injection / self.half_life_minutes);
        let dose_bq = self.dose_mbq * 1e6 * decay;
        Ok(self.bq_ml_per_unit * self.weight_kg * 1000.0 / dose_bq)
    }

    pub fn from_sidecar(volume: &Path) -> Option<Result<Self>> {
        let path = sidecar_path(volume);
        let text = std::fs::read_to_string(&path).ok()?;
        Some(
            Self::from_json(&text)
                .map_err(|error| Error::invalid(format!("{}: {error}", path.display()))),
        )
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let json: Value = serde_json::from_str(text)
            .map_err(|error| Error::invalid(format!("Invalid JSON: {error}")))?;
        let number = |key: &str| json.get(key).and_then(Value::as_f64);
        let text = |key: &str| json.get(key).and_then(Value::as_str);
        let missing = |what: &str| Error::invalid(format!("The sidecar has no {what}"));

        let weight_kg = number("BodyWeight")
            .or(number("PatientWeight"))
            .ok_or_else(|| missing("body weight"))?;
        let dose_mbq = match number("InjectedRadioactivity") {
            Some(dose) => dose * mbq_per(text("InjectedRadioactivityUnits").unwrap_or("MBq"))?,
            None => number("RadionuclideTotalDose")
                .map(|bq| bq / 1e6)
                .ok_or_else(|| missing("injected dose"))?,
        };
        let half_life_minutes = match number("RadionuclideHalfLife") {
            Some(seconds) => seconds / 60.0,
            None => text("TracerRadionuclide")
                .and_then(half_life_of)
                .ok_or_else(|| missing("known tracer or half-life"))?,
        };
        let bq_ml_per_unit = match text("Units") {
            None => 1.0,
            Some(unit) => match unit.to_ascii_lowercase().replace('/', "").as_str() {
                "bqml" => 1.0,
                "kbqml" => 1e3,
                "mbqml" => 1e6,
                _ => {
                    return Err(Error::invalid(format!(
                        "The image is in {unit}, not an activity (like Bq/mL)"
                    )));
                }
            },
        };
        Ok(Self {
            weight_kg,
            dose_mbq,
            minutes_after_injection: decay_minutes(&json)?,
            half_life_minutes,
            bq_ml_per_unit,
        })
    }
}

fn decay_minutes(json: &Value) -> Result<f64> {
    let number = |key: &str| json.get(key).and_then(Value::as_f64);
    let text = |key: &str| json.get(key).and_then(Value::as_str);
    let uncorrected =
        || Error::invalid("The image isn't decay corrected, so SUV depends on the frame time");
    if json.get("ImageDecayCorrected").and_then(Value::as_bool) == Some(false) {
        return Err(uncorrected());
    }
    if let (Some(corrected), Some(injected)) =
        (number("ImageDecayCorrectionTime"), number("InjectionStart"))
    {
        return Ok((corrected - injected) / 60.0);
    }
    match text("DecayCorrection") {
        Some("ADMIN") => return Ok(0.0),
        Some("NONE") => return Err(uncorrected()),
        Some("START") => {
            let start = text("AcquisitionTime")
                .or(text("SeriesTime"))
                .and_then(clock_seconds);
            let injection = text("RadiopharmaceuticalStartTime").and_then(clock_seconds);
            if let (Some(start), Some(injection)) = (start, injection) {
                // a scan after midnight
                let seconds = (start - injection).rem_euclid(24.0 * 3600.0);
                return Ok(seconds / 60.0);
            }
        }
        _ => {}
    }
    Err(Error::invalid(
        "The sidecar has no time between injection and scan",
    ))
}

fn clock_seconds(text: &str) -> Option<f64> {
    let digits: String = text.chars().filter(|c| *c != ':').collect();
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    if whole.len() != 6 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |range: std::ops::Range<usize>| whole[range].parse::<f64>().ok();
    let fraction = format!("0.{fraction}").parse::<f64>().unwrap_or(0.0);
    Some(part(0..2)? * 3600.0 + part(2..4)? * 60.0 + part(4..6)? + fraction)
}

fn mbq_per(unit: &str) -> Result<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "bq" => Ok(1e-6),
        "kbq" => Ok(1e-3),
        "mbq" => Ok(1.0),
        "gbq" => Ok(1e3),
        "mci" => Ok(37.0),
        "uci" | "µci" => Ok(0.037),
        _ => Err(Error::invalid(format!("Unknown dose unit {unit}"))),
    }
}

pub fn half_life_of(name: &str) -> Option<f64> {
    let key = |name: &str| {
        let letters: String = name
            .chars()
            .filter(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_uppercase();
        let digits: String = name.chars().filter(char::is_ascii_digit).collect();
        (letters, digits)
    };
    let wanted = key(name);
    TRACERS
        .iter()
        .find(|(tracer, _)| key(tracer) == wanted)
        .map(|&(_, minutes)| minutes)
}

pub fn sidecar_path(volume: &Path) -> PathBuf {
    let name = volume
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let suffixes = [
        ".nii.gz", ".nii", ".nrrd", ".nhdr", ".hdr", ".img.gz", ".img",
    ];
    let stem = suffixes
        .iter()
        .find(|suffix| lower.ends_with(*suffix))
        .map_or(name.as_str(), |suffix| &name[..name.len() - suffix.len()]);
    volume.with_file_name(format!("{stem}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * a.abs().max(1.0)
    }

    #[test]
    fn suv_scales_by_weight_and_decayed_dose() {
        let inputs = SuvInputs {
            weight_kg: 70.0,
            dose_mbq: 350.0,
            minutes_after_injection: 0.0,
            half_life_minutes: 109.77,
            bq_ml_per_unit: 1.0,
        };
        assert!(close(inputs.factor().unwrap() * 5000.0, 1.0));
        let later = SuvInputs {
            minutes_after_injection: 109.77,
            ..inputs
        };
        assert!(close(
            later.factor().unwrap(),
            2.0 * inputs.factor().unwrap()
        ));
        let weightless = SuvInputs {
            weight_kg: 0.0,
            ..inputs
        };
        assert!(weightless.factor().is_err());
    }

    #[test]
    fn reads_bids_sidecars() {
        let inputs = SuvInputs::from_json(
            r#"{
                "BodyWeight": 80,
                "InjectedRadioactivity": 9.46,
                "InjectedRadioactivityUnits": "mCi",
                "TracerRadionuclide": "F18",
                "Units": "kBq/mL",
                "ImageDecayCorrected": true,
                "InjectionStart": 0,
                "ImageDecayCorrectionTime": 3600
            }"#,
        )
        .unwrap();
        assert_eq!(inputs.weight_kg, 80.0);
        assert!(close(inputs.dose_mbq, 9.46 * 37.0));
        assert_eq!(inputs.minutes_after_injection, 60.0);
        assert_eq!(inputs.half_life_minutes, 109.77);
        assert_eq!(inputs.bq_ml_per_unit, 1000.0);

        let uncorrected = r#"{"BodyWeight": 80, "InjectedRadioactivity": 350,
            "TracerRadionuclide": "F18", "ImageDecayCorrected": false}"#;
        assert!(SuvInputs::from_json(uncorrected).is_err());
        let already_suv = r#"{"BodyWeight": 80, "InjectedRadioactivity": 350,
            "TracerRadionuclide": "F18", "Units": "g/mL", "InjectionStart": 0,
            "ImageDecayCorrectionTime": 0}"#;
        assert!(SuvInputs::from_json(already_suv).is_err());
    }

    #[test]
    fn reads_dcm2niix_sidecars() {
        let inputs = SuvInputs::from_json(
            r#"{
                "PatientWeight": 72.5,
                "RadionuclideTotalDose": 314000000,
                "RadionuclideHalfLife": 6586.2,
                "Units": "BQML",
                "DecayCorrection": "START",
                "RadiopharmaceuticalStartTime": "23:50:00.000000",
                "AcquisitionTime": "00:50:30.500000"
            }"#,
        )
        .unwrap();
        assert_eq!(inputs.weight_kg, 72.5);
        assert_eq!(inputs.dose_mbq, 314.0);
        assert!(close(inputs.half_life_minutes, 109.77));
        // across midnight: one hour and half a minute
        assert!(close(inputs.minutes_after_injection, 60.0 + 30.5 / 60.0));
        assert_eq!(inputs.bq_ml_per_unit, 1.0);
    }

    #[test]
    fn finds_tracers_and_sidecars() {
        for name in ["F18", "18F", "F-18", "[18F]", "f18"] {
            assert_eq!(half_life_of(name), Some(109.77), "{name}");
        }
        assert_eq!(half_life_of("Ga68"), Some(67.71));
        assert_eq!(half_life_of("Tc99m"), None);
        assert_eq!(
            sidecar_path(Path::new("/data/sub-1_pet.nii.gz")),
            PathBuf::from("/data/sub-1_pet.json")
        );
        assert_eq!(sidecar_path(Path::new("ct.nrrd")), PathBuf::from("ct.json"));
    }
}
