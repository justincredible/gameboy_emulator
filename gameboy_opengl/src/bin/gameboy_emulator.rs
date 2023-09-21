#[macro_use]
extern crate clap;

use clap::{App, Arg};
use std::fs::File;
use std::io::Read;
use std::process::{Command, Stdio};

fn main() -> Result<(), String> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .args(&[
            Arg::with_name("rom filename")
                .help("rom file to use")
                .required(true)
                .index(1),
            Arg::with_name("linked filename")
                .help("rom file to link with")
                .required(false)
                .index(2),
        ])
        .get_matches();

    let child = if let Some(linked_filename) = matches.value_of("linked filename") {
        let bin_name = std::env::args_os().next().unwrap();

        Command::new(&bin_name)
            .arg(&linked_filename)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()
    }
    let rom_filename = matches.value_of("rom filename").unwrap();
    let mut file = File::open(rom_filename).map_err(|e| format!("{:?}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| format!("{:?}", e))?;
    gameboy_opengl::start(buffer, child)?;

    Ok(())
}
