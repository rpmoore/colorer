mod cli;
mod color;
mod commands;
mod device;

use clap::Parser;
use cli::{Cli, Commands};
use commands::list::run_list;
use commands::set::run_set;
use commands::show::run_show;
use device::ColorWriter;
use device::DeviceBackend;
use device::hid::HidBackend;
use device::sysfs::SysfsBackend;

fn backends() -> Vec<Box<dyn DeviceBackend>> {
    vec![
        Box::new(HidBackend::new()),
        Box::new(SysfsBackend::new("/sys/class/leds")),
    ]
}

// sysfs write support lands in section-06; HID is the only writer for now.
fn color_writers() -> Vec<Box<dyn ColorWriter>> {
    vec![Box::new(HidBackend::new())]
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::List(args) => match run_list(&backends(), args.all) {
            Ok(output) => println!("{output}"),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
        Commands::Show(args) => match run_show(&backends(), &args.id) {
            Ok(output) => println!("{output}"),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
        Commands::Set(args) => match run_set(&color_writers(), &args.id, &args.color) {
            Ok(output) => println!("{output}"),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
    }
}
