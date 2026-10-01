use std::path::Path;

use illuminator_core::geometry::{GeometryOptions, VolumeGeometry};
use illuminator_core::header::Header;

fn main() {
    for arg in std::env::args().skip(1) {
        let header = match Header::read(Path::new(&arg)) {
            Ok(header) => header,
            Err(error) => {
                eprintln!("{arg}: {error}");
                continue;
            }
        };
        println!("{arg}\n{header:#?}");
        match VolumeGeometry::new(&header, GeometryOptions::default()) {
            Ok(geometry) => println!("{geometry:#?}\n"),
            Err(error) => eprintln!("{arg}: geometry: {error}\n"),
        }
    }
}
