//! SUV calculations for PET.
//!
//! The inputs can come from a JSON sidecar.

use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::metadata::strip_extension;

/// Tracer half-life in minutes from <https://www.nndc.bnl.gov/nudat3/indx_dec.jsp>
pub const TRACER_HALF_LIVES: [(&str, f64); 9] = [
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiologicalSex {
    Male,
    Female,
}

impl BiologicalSex {
    pub fn label(self) -> &'static str {
        match self {
            Self::Female => "Female",
            Self::Male => "Male",
        }
    }

    fn from_code(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "M" | "MALE" => Some(Self::Male),
            "F" | "FEMALE" => Some(Self::Female),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Normalization {
    #[default]
    BodyWeight,
    LbmJames,
    LbmJanma,
}

impl Normalization {
    pub const ALL: [Self; 3] = [Self::BodyWeight, Self::LbmJames, Self::LbmJanma];

    pub fn label(self) -> &'static str {
        match self {
            Self::BodyWeight => "BW",
            Self::LbmJames => "LBM (James)",
            Self::LbmJanma => "LBM (Janma)",
        }
    }

    pub fn suv_name(self) -> &'static str {
        match self {
            Self::BodyWeight => "SUV-BW",
            Self::LbmJames => "SUV-LBM (James)",
            Self::LbmJanma => "SUV-LBM (Janma)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuvInputs {
    pub weight_kg: f64,
    pub height_cm: Option<f64>,
    pub biological_sex: Option<BiologicalSex>,
    pub normalization: Normalization,
    pub dose_mbq: f64,
    pub minutes_after_injection: f64,
    pub half_life_minutes: f64,
    pub bq_ml_per_unit: f64,
}

impl SuvInputs {
    pub fn conversion_factor(&self) -> Result<f64> {
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
        Ok(self.bq_ml_per_unit * self.normalization_mass_kg()? * 1000.0 / dose_bq)
    }

    /// BW or LBM depending on normalization.
    pub fn normalization_mass_kg(&self) -> Result<f64> {
        let weight = self.weight_kg;
        let height_and_sex = || match (self.height_cm, self.biological_sex) {
            (Some(height), Some(biological_sex)) if height.is_finite() && height > 0.0 => {
                Ok((height, biological_sex))
            }
            _ => Err(Error::invalid(
                "Lean body mass needs a height and biological sex",
            )),
        };
        let lean_body_mass = match self.normalization {
            Normalization::BodyWeight => return Ok(weight),
            Normalization::LbmJames => {
                let (height, biological_sex) = height_and_sex()?;
                let (a, b) = match biological_sex {
                    BiologicalSex::Female => (1.07, 148.0),
                    BiologicalSex::Male => (1.1, 128.0),
                };
                a * weight - b * (weight / height).powi(2)
            }
            Normalization::LbmJanma => {
                let (height, biological_sex) = height_and_sex()?;
                let bmi = weight / (height / 100.0).powi(2);
                let (a, b) = match biological_sex {
                    BiologicalSex::Female => (8.78e3, 244.0),
                    BiologicalSex::Male => (6.68e3, 216.0),
                };
                9.27e3 * weight / (a + b * bmi)
            }
        };
        // the LBMJames parabola goes negative at a very high BMI
        if lean_body_mass > 0.0 {
            Ok(lean_body_mass)
        } else {
            Err(Error::invalid(
                "Lean body mass isn't positive at this weight and height",
            ))
        }
    }

    pub fn from_sidecar(volume: &Path) -> Option<Result<Self>> {
        let path = sidecar_path(volume);
        let text = std::fs::read_to_string(&path).ok()?;
        Some(
            Self::from_json(&text)
                .map_err(|error| Error::invalid(format!("{}: {error}", path.display()))),
        )
    }

    pub(crate) fn from_json(text: &str) -> Result<Self> {
        let sidecar = Sidecar(
            serde_json::from_str(text)
                .map_err(|error| Error::invalid(format!("Invalid JSON: {error}")))?,
        );
        let missing = |what: &str| Error::invalid(format!("The sidecar has no {what}"));

        let weight_kg = sidecar
            .number("BodyWeight")
            .or(sidecar.number("PatientWeight"))
            .ok_or_else(|| missing("body weight"))?;
        let dose_mbq = match sidecar.number("InjectedRadioactivity") {
            Some(dose) => {
                dose * megabecquerels_factor(
                    sidecar.text("InjectedRadioactivityUnits").unwrap_or("MBq"),
                )?
            }
            None => sidecar
                .number("RadionuclideTotalDose")
                .map(|bq| bq / 1e6)
                .ok_or_else(|| missing("injected dose"))?,
        };
        let half_life_minutes = match sidecar.number("RadionuclideHalfLife") {
            Some(seconds) => seconds / 60.0,
            None => sidecar
                .text("TracerRadionuclide")
                .and_then(half_life_of)
                .ok_or_else(|| missing("known tracer or half-life"))?,
        };
        let bq_ml_per_unit = match sidecar.text("Units") {
            Some(unit) => becquerels_per_ml_factor(unit)?,
            None => 1.0,
        };
        let height_cm = sidecar
            .number("PatientSize")
            .map(|size| if size < 3.0 { size * 100.0 } else { size });
        let biological_sex = sidecar
            .text("PatientSex")
            .and_then(BiologicalSex::from_code);
        Ok(Self {
            weight_kg,
            height_cm,
            biological_sex,
            normalization: Normalization::BodyWeight,
            dose_mbq,
            minutes_after_injection: minutes_since_injection(&sidecar)?,
            half_life_minutes,
            bq_ml_per_unit,
        })
    }
}

fn minutes_since_injection(sidecar: &Sidecar) -> Result<f64> {
    let uncorrected =
        || Error::invalid("The image isn't decay corrected, so SUV depends on the frame time");
    if sidecar.flag("ImageDecayCorrected") == Some(false) {
        return Err(uncorrected());
    }
    if let (Some(corrected), Some(injected)) = (
        sidecar.number("ImageDecayCorrectionTime"),
        sidecar.number("InjectionStart"),
    ) {
        return Ok((corrected - injected) / 60.0);
    }
    match sidecar.text("DecayCorrection") {
        Some("ADMIN") => return Ok(0.0),
        Some("NONE") => return Err(uncorrected()),
        Some("START") => {
            let start = sidecar
                .text("AcquisitionTime")
                .or(sidecar.text("SeriesTime"))
                .and_then(seconds_since_midnight);
            let injection = sidecar
                .text("RadiopharmaceuticalStartTime")
                .and_then(seconds_since_midnight);
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

/// BIDS or dcm2niix sidecar
struct Sidecar(Value);

impl Sidecar {
    fn number(&self, key: &str) -> Option<f64> {
        self.0.get(key).and_then(Value::as_f64)
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    fn flag(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }
}

fn seconds_since_midnight(text: &str) -> Option<f64> {
    let digits: String = text.chars().filter(|c| *c != ':').collect();
    let (whole, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
    if whole.len() != 6 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |range: std::ops::Range<usize>| whole[range].parse::<f64>().ok();
    let fraction = format!("0.{fraction}").parse::<f64>().unwrap_or(0.0);
    Some(part(0..2)? * 3600.0 + part(2..4)? * 60.0 + part(4..6)? + fraction)
}

/// How many Bq/mL one unit of the image is
fn becquerels_per_ml_factor(unit: &str) -> Result<f64> {
    match unit.to_ascii_lowercase().replace('/', "").as_str() {
        "bqml" => Ok(1.0),
        "kbqml" => Ok(1e3),
        "mbqml" => Ok(1e6),
        _ => Err(Error::invalid(format!(
            "The image is in {unit}. Should be an activity (e.g. Bq/mL)"
        ))),
    }
}

fn megabecquerels_factor(unit: &str) -> Result<f64> {
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

pub(crate) fn half_life_of(name: &str) -> Option<f64> {
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
    TRACER_HALF_LIVES
        .iter()
        .find(|(tracer, _)| key(tracer) == wanted)
        .map(|&(_, minutes)| minutes)
}

pub(crate) fn sidecar_path(volume: &Path) -> PathBuf {
    let name = volume
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    volume.with_file_name(format!("{}.json", strip_extension(&name)))
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
            height_cm: None,
            biological_sex: None,
            normalization: Normalization::BodyWeight,
            dose_mbq: 350.0,
            minutes_after_injection: 0.0,
            half_life_minutes: 109.77,
            bq_ml_per_unit: 1.0,
        };
        assert!(close(inputs.conversion_factor().unwrap() * 5000.0, 1.0));
        let later = SuvInputs {
            minutes_after_injection: 109.77,
            ..inputs
        };
        assert!(close(
            later.conversion_factor().unwrap(),
            2.0 * inputs.conversion_factor().unwrap()
        ));
        let weightless = SuvInputs {
            weight_kg: 0.0,
            ..inputs
        };
        assert!(weightless.conversion_factor().is_err());
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
