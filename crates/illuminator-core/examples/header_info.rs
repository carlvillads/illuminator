use illuminator_core::header::Header;
use std::path::Path;

fn main() {
    for arg in std::env::args().skip(1) {
        match Header::read(Path::new(&arg)) {
            Ok(header) => println!("{arg}\n{header:#?}\n"),
            Err(error) => eprintln!("{arg}: {error}"),
        }
    }
}
