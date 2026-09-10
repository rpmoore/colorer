mod cli;
mod commands;
mod device;

use clap::Parser;
use cli::{Cli, Commands};
use commands::list::run_list;
use device::DeviceBackend;
use device::hid::HidBackend;
use device::sysfs::SysfsBackend;

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::List(args) => {
            let backends: Vec<Box<dyn DeviceBackend>> = vec![
                Box::new(HidBackend::new()),
                Box::new(SysfsBackend::new("/sys/class/leds")),
            ];
            match run_list(&backends, args.all) {
                Ok(output) => println!("{output}"),
                Err(err) => {
                    eprintln!("error: {err}");
                    std::process::exit(1);
                }
            }
        }
    }
}
