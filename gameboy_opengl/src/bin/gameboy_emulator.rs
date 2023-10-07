#[macro_use]
extern crate clap;

use clap::{App, Arg};
use std::fs::File;
use std::io::Read;
use std::process::Command;

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
            Arg::from_usage("[link] -l, --link <rom> 'rom file to link with'"),
            Arg::from_usage("[linked] --linked 'Run with linking'")
                .hidden(true),
        ])
        .get_matches();

    let linked = (
        matches.is_present("linked"),
        matches
            .value_of("link")
            .and_then(|linked_filename| {
                let bin_name = std::env::args_os().next().unwrap();

                Command::new(&bin_name)
                    .arg(&linked_filename)
                    .arg("--linked")
                    .spawn()
                    .ok()
            })
    );
    let rom_filename = matches.value_of("rom filename").unwrap();
    let mut file = File::open(rom_filename).map_err(|e| format!("{:?}", e))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| format!("{:?}", e))?;
    gameboy_opengl::start(buffer, linked)?;

    Ok(())
}
