use mkfs::{Command, USAGE};

fn main() {
    let command = match Command::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let Command::Build(config) = command else {
        println!("{USAGE}");
        return;
    };

    match mkfs::build_image(&config) {
        Ok(summary) => {
            for path in &summary.skipped {
                eprintln!("warning: skipping unsupported entry {}", path.display());
            }

            println!(
                "Created {} from {} ({} directories, {} files, {} skipped; {} blocks, {} bytes)",
                config.output.display(),
                config.source.display(),
                summary.directories,
                summary.files,
                summary.skipped.len(),
                config.total_blocks,
                config.total_blocks * filesystem::BLOCK_SIZE,
            );
        }
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}
