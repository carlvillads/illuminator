//! Reads NRRD files. See <https://teem.sourceforge.net/nrrd/format.html>
//! Should support `.nrrd` and `.nhdr` files.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::{Error, Result};
use crate::geometry::Point3D;
use crate::meta::{DataFile, Format, Mat3x4, SpatialUnits, Transform, VolumeMeta};
use crate::voxel::{self, DataType, Endian, Scaling};

const MAX_HDR_BYTES: u64 = 1 << 20;

pub fn read(path: &Path) -> Result<VolumeMeta> {
    let header = Header::read(path)?;
    parse(&header, path).map_err(|error| match error {
        Error::Invalid(reason) => Error::invalid(format!("{}: {reason}", path.display())),
        other => other,
    })
}

struct Header {
    fields: HashMap<String, String>,
    length: u64,
}

impl Header {
    fn read(path: &Path) -> Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let mut fields = HashMap::new();
        let mut length = 0;
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = file.read_until(b'\n', &mut line)?;
            length += read as u64;
            if length > MAX_HDR_BYTES {
                return Err(Error::invalid("NRRD header is too long"));
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches(['\n', '\r']);
            if length == read as u64 {
                if !text.starts_with("NRRD000") {
                    return Err(Error::invalid("Not a NRRD file"));
                }
                continue;
            }
            if read == 0 || text.is_empty() {
                return Ok(Self { fields, length });
            }
            if text.starts_with('#') || text.contains(":=") {
                continue;
            }
            if let Some((name, value)) = text.split_once(": ") {
                fields.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
    }

    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str> {
        self.field(name)
            .ok_or_else(|| Error::invalid(format!("The header has no \"{name}\" field")))
    }
}

fn parse(header: &Header, path: &Path) -> Result<VolumeMeta> {
    let datatype = data_type(header.required("type")?)?;
    let dimension: usize = number(header.required("dimension")?)?;
    if !(2..=4).contains(&dimension) {
        return Err(Error::invalid("Expected a 2D, 3D, or 4D image"));
    }
    let sizes = header
        .required("sizes")?
        .split_whitespace()
        .map(number)
        .collect::<Result<Vec<usize>>>()?;
    if sizes.len() != dimension || sizes.contains(&0) {
        return Err(Error::invalid("\"sizes\" doesn't match \"dimension\""));
    }
    let dims: [usize; 4] = std::array::from_fn(|i| sizes.get(i).copied().unwrap_or(1));
    let bytes = dims.iter().try_fold(datatype.size(), |count, &size| {
        voxel::checked_mul(count, size)
    })?;

    let gzip = match header.required("encoding")? {
        "raw" => false,
        "gzip" | "gz" => true,
        other => {
            return Err(Error::invalid(format!(
                "The {other} encoding isn't supported (yet)"
            )));
        }
    };
    let endian = match header.field("endian") {
        Some("little") => Endian::Little,
        Some("big") => Endian::Big,
        // one byte per voxel has no byte order
        None if datatype.size() == 1 => Endian::Little,
        _ => return Err(Error::invalid("The header has no valid \"endian\" field")),
    };

    let (spacing, units, transform) = match header.field("space directions") {
        Some(directions) => space(header, directions, dimension)?,
        None => {
            // without a space, the spacing has no known unit
            let spacings: Vec<f64> = header
                .field("spacings")
                .map(|text| {
                    text.split_whitespace()
                        .map(|value| value.parse().unwrap_or(1.0))
                        .collect()
                })
                .unwrap_or_default();
            let spacing = std::array::from_fn(|i| spacings.get(i).copied().unwrap_or(1.0));
            (spacing, SpatialUnits::Unknown, None)
        }
    };

    let (data_path, start) = match header.field("data file").or(header.field("datafile")) {
        Some(name) if name.starts_with("LIST") || name.contains('%') => {
            return Err(Error::invalid(
                "Data split over several files isn't supported (yet)",
            ));
        }
        Some(name) => (path.with_file_name(name), 0),
        None => (path.to_path_buf(), header.length),
    };
    if header.field("line skip").is_some_and(|skip| skip != "0") {
        return Err(Error::invalid("\"line skip\" isn't supported (yet)"));
    }
    let offset = match header.field("byte skip").unwrap_or("0") {
        // -1: the data is at the end of the file
        "-1" if !gzip => {
            let file_bytes = std::fs::metadata(&data_path)?.len();
            file_bytes
                .checked_sub(start + bytes as u64)
                .ok_or_else(|| Error::invalid("The data file is too short"))?
        }
        skip => number(skip)?,
    };

    Ok(VolumeMeta {
        format: Format::Nrrd,
        dims,
        spacing,
        units,
        transform,
        datatype,
        endian,
        scaling: Scaling::default(),
        display_range: None,
        data: DataFile {
            path: data_path,
            start,
            offset,
            bytes,
            gzip,
        },
    })
}

fn space(
    header: &Header,
    directions: &str,
    dimension: usize,
) -> Result<([f64; 3], SpatialUnits, Option<Transform>)> {
    let directions = vectors(directions)?;
    if directions.len() != dimension {
        return Err(Error::invalid(
            "\"space directions\" doesn't match \"dimension\"",
        ));
    }
    let spatial = directions
        .iter()
        .take_while(|vector| vector.is_some())
        .count();
    if spatial != dimension.min(3) || directions[spatial..].iter().any(Option::is_some) {
        return Err(Error::invalid("Vector images aren't supported (yet)"));
    }
    let mut columns: Vec<[f64; 3]> = directions.into_iter().flatten().collect();
    if columns.len() == 2 {
        let normal = Point3D::from_array(columns[0]).cross(Point3D::from_array(columns[1]));
        let length = normal.dot(normal).sqrt();
        columns.push([normal.x / length, normal.y / length, normal.z / length]);
    }

    let flip = match header.field("space") {
        Some("right-anterior-superior" | "RAS" | "scanner-xyz" | "3D-right-handed") => [1.0; 3],
        Some("left-posterior-superior" | "LPS") => [-1.0, -1.0, 1.0],
        Some("left-anterior-superior" | "LAS") => [-1.0, 1.0, 1.0],
        Some(other) => {
            return Err(Error::invalid(format!(
                "The \"{other}\" space isn't supported"
            )));
        }
        None if header.field("space dimension") == Some("3") => [1.0; 3],
        None => return Err(Error::invalid("\"space directions\" without a \"space\"")),
    };
    let origin = match header.field("space origin") {
        Some(text) => vectors(text)?
            .first()
            .copied()
            .flatten()
            .ok_or_else(|| Error::invalid("Invalid \"space origin\""))?,
        None => [0.0; 3],
    };
    let matrix: Mat3x4 = std::array::from_fn(|row| {
        let [x, y, z] = [0, 1, 2].map(|axis| columns[axis][row]);
        [x, y, z, origin[row]].map(|value| value * flip[row])
    });
    let spacing = [0, 1, 2].map(|axis| columns[axis].iter().map(|x| x * x).sum::<f64>().sqrt());

    let units = header
        .field("space units")
        .map_or(SpatialUnits::Millimeter, parse_spatial_units);
    let transform = Transform {
        name: "space directions",
        matrix,
    };
    Ok((spacing, units, Some(transform)))
}

fn parse_spatial_units(text: &str) -> SpatialUnits {
    let mut units = text.split_whitespace().map(|unit| unit.trim_matches('"'));
    let Some(first) = units.next() else {
        return SpatialUnits::Unknown;
    };
    if units.any(|unit| unit != first) {
        return SpatialUnits::Unknown;
    }
    match first {
        "mm" => SpatialUnits::Millimeter,
        "m" => SpatialUnits::Meter,
        "um" | "µm" => SpatialUnits::Micron,
        _ => SpatialUnits::Unknown,
    }
}

fn vectors(text: &str) -> Result<Vec<Option<[f64; 3]>>> {
    let invalid = || Error::invalid(format!("Invalid vectors \"{text}\""));
    let mut vectors = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("none") {
            vectors.push(None);
            rest = after.trim_start();
            continue;
        }
        let end = rest.find(')').ok_or_else(invalid)?;
        let inside = rest.strip_prefix('(').ok_or_else(invalid)?;
        let values = inside[..end - 1]
            .split(',')
            .map(|value| value.trim().parse::<f64>().map_err(|_| invalid()))
            .collect::<Result<Vec<f64>>>()?;
        let vector: [f64; 3] = values.try_into().map_err(|_| invalid())?;
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(invalid());
        }
        vectors.push(Some(vector));
        rest = rest[end + 1..].trim_start();
    }
    Ok(vectors)
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T> {
    text.trim()
        .parse()
        .map_err(|_| Error::invalid(format!("Invalid number \"{text}\"")))
}

fn data_type(name: &str) -> Result<DataType> {
    // NRRD allows several spellings of each type
    Ok(match name {
        "signed char" | "int8" | "int8_t" => DataType::I8,
        "uchar" | "unsigned char" | "uint8" | "uint8_t" => DataType::U8,
        "short" | "short int" | "signed short" | "signed short int" | "int16" | "int16_t" => {
            DataType::I16
        }
        "ushort" | "unsigned short" | "unsigned short int" | "uint16" | "uint16_t" => DataType::U16,
        "int" | "signed int" | "int32" | "int32_t" => DataType::I32,
        "uint" | "unsigned int" | "uint32" | "uint32_t" => DataType::U32,
        "float" => DataType::F32,
        "double" => DataType::F64,
        other => {
            return Err(Error::invalid(format!(
                "The voxel type \"{other}\" isn't supported (yet)"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;
    use crate::cache::VolumeCache;
    use crate::progress::Progress;
    use crate::volume::Volume;

    const LPS: &str = "space: left-posterior-superior
space directions: (2,0,0) (0,3,0) (0,0,4)
space origin: (10,20,30)";

    fn voxels(big_endian: bool) -> Vec<u8> {
        (0..48i16)
            .flat_map(|i| {
                if big_endian {
                    i.to_be_bytes()
                } else {
                    i.to_le_bytes()
                }
            })
            .collect()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn header(fields: &str) -> Vec<u8> {
        format!("NRRD0004\n# made by a test\n{fields}\n\n").into_bytes()
    }

    fn write(dir: &tempfile::TempDir, name: &str, parts: &[&[u8]]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, parts.concat()).unwrap();
        path
    }

    fn load(path: &Path) -> Volume {
        let cache = VolumeCache::new(path.with_file_name("cache"), 1 << 20).unwrap();
        Volume::load(path, &cache, &Progress::default()).unwrap()
    }

    #[test]
    fn reads_attached_raw_data() {
        let dir = tempfile::tempdir().unwrap();
        let fields = format!(
            "type: short\ndimension: 3\nsizes: 4 3 4\nencoding: raw\nendian: little\n{LPS}"
        );
        let head = header(&fields);
        let path = write(&dir, "ct.nrrd", &[&head, &voxels(false)]);

        let meta = VolumeMeta::read(&path).unwrap();
        assert_eq!(meta.format, Format::Nrrd);
        assert_eq!(meta.dims, [4, 3, 4, 1]);
        assert_eq!(meta.spacing, [2.0, 3.0, 4.0]);
        assert_eq!(meta.units, SpatialUnits::Millimeter);
        assert_eq!(
            meta.transform.unwrap().matrix,
            [
                [-2.0, 0.0, 0.0, -10.0],
                [0.0, -3.0, 0.0, -20.0],
                [0.0, 0.0, 4.0, 30.0]
            ]
        );
        assert_eq!((meta.data.start, meta.data.offset), (head.len() as u64, 0));
        assert_eq!(load(&path).sample_value([3, 2, 3], 0), Some(47.0));
    }

    #[test]
    fn reads_attached_gzip_data() {
        let dir = tempfile::tempdir().unwrap();
        let head =
            header("type: int16\ndimension: 3\nsizes: 4 3 4\nencoding: gzip\nendian: little");
        let path = write(&dir, "ct.nrrd", &[&head, &gzip(&voxels(false))]);
        assert_eq!(load(&path).sample_value([3, 2, 3], 0), Some(47.0));
    }

    #[test]
    fn reads_detached_data() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir, "ct.raw.gz", &[&gzip(&voxels(true))]);
        let fields = format!(
            "type: ushort\ndimension: 3\nsizes: 4 3 4\nencoding: gz\nendian: big\ndata file: ct.raw.gz\n{LPS}"
        );
        let mut head = header(&fields);
        head.pop();
        let path = write(&dir, "ct.nhdr", &[&head]);
        let volume = load(&path);
        assert_eq!(volume.meta().data.path, dir.path().join("ct.raw.gz"));
        assert_eq!(volume.sample_value([3, 2, 3], 0), Some(47.0));

        write(&dir, "end.raw", &[b"junk", &voxels(false)]);
        let head = header(
            "type: short\ndimension: 3\nsizes: 4 3 4\nencoding: raw\nendian: little\ndata file: end.raw\nbyte skip: -1",
        );
        let path = write(&dir, "end.nhdr", &[&head]);
        assert_eq!(load(&path).sample_value([3, 2, 3], 0), Some(47.0));
    }

    #[test]
    fn a_fourth_axis_without_direction_is_time() {
        let dir = tempfile::tempdir().unwrap();
        let space = "space: RAS\nspace directions: (1,0,0) (0,1,0) (0,0,1) none";
        let fields = format!("type: uchar\ndimension: 4\nsizes: 2 2 2 3\nencoding: raw\n{space}");
        let path = write(&dir, "pet.nrrd", &[&header(&fields), &[0; 24]]);
        assert_eq!(VolumeMeta::read(&path).unwrap().timepoints(), 3);

        let space = "space: RAS\nspace directions: none (1,0,0) (0,1,0) (0,0,1)";
        let fields = format!("type: uchar\ndimension: 4\nsizes: 3 2 2 2\nencoding: raw\n{space}");
        let path = write(&dir, "dwi.nrrd", &[&header(&fields), &[0; 24]]);
        assert!(VolumeMeta::read(&path).is_err());
    }

    #[test]
    fn space_units_must_be_the_same_on_every_axis() {
        assert_eq!(
            parse_spatial_units(r#""mm" "mm" "mm""#),
            SpatialUnits::Millimeter
        );
        assert_eq!(parse_spatial_units(r#""um" "um""#), SpatialUnits::Micron);
        assert_eq!(
            parse_spatial_units(r#""mm" "m" "mm""#),
            SpatialUnits::Unknown
        );
        assert_eq!(parse_spatial_units(""), SpatialUnits::Unknown);
    }

    #[test]
    fn without_space_only_spacing_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let fields = "type: float\ndimension: 3\nsizes: 2 2 2\nencoding: raw\nendian: little\nspacings: 0.5 0.5 2";
        let path = write(&dir, "plain.nrrd", &[&header(fields), &[0; 32]]);
        let meta = VolumeMeta::read(&path).unwrap();
        assert_eq!(meta.transform, None);
        assert_eq!(meta.spacing, [0.5, 0.5, 2.0]);
        assert_eq!(meta.units, SpatialUnits::Unknown);
    }

    #[test]
    fn rejects_what_it_cannot_show() {
        let dir = tempfile::tempdir().unwrap();
        let base = "dimension: 3\nsizes: 2 2 2\nencoding: raw\nendian: little";
        for (name, fields) in [
            ("no-type", base.to_string()),
            ("int64", format!("type: int64\n{base}")),
            (
                "ascii",
                format!("type: short\n{}", base.replace("raw", "ascii")),
            ),
            (
                "sizes",
                format!("type: short\n{}", base.replace("2 2 2", "2 2")),
            ),
            (
                "endian",
                format!("type: short\n{}", base.replace("endian: little", "")),
            ),
            (
                "space",
                format!("type: short\n{base}\nspace directions: (1,0,0) (0,1,0) (0,0,1)"),
            ),
        ] {
            let path = write(&dir, &format!("{name}.nrrd"), &[&header(&fields), &[0; 16]]);
            assert!(VolumeMeta::read(&path).is_err(), "{name}");
        }
        let path = write(&dir, "not.nrrd", &[b"P5 2 2 255\n"]);
        assert!(VolumeMeta::read(&path).is_err());
    }
}
