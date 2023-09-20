#[macro_use]
extern crate clap;

use clap::{App, Arg};
use std::fs::File;
use std::io::Read;

fn main() -> Result<(), String> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::with_name("rom filename")
                .help("rom file to use")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("linked rom filename")
                .help("rom file to link with")
                .required(false)
                .index(2),
        )
        .get_matches();

    let rom_filename = matches.value_of("rom filename").unwrap();
    if let Some(filename) = matches.value("linked rom filename") {

    }
    let mut file = File::open(rom_filename).map_err(|e| format!("{:?}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| format!("{:?}", e))?;
    gameboy_opengl::start(buffer)?;

    Ok(())
}
