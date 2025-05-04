use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    path: std::path::PathBuf,
    path2: std::path::PathBuf,
}

fn main() {
    let args = Args::parse();
    println!("{:?}", args.path.into_os_string());
}
